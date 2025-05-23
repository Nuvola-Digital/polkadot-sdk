// Copyright (C) Parity Technologies (UK) Ltd.
// This file is part of Polkadot.

// Polkadot is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Polkadot is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Polkadot.  If not, see <http://www.gnu.org/licenses/>.

//! Utilities and tests for locating the PVF worker binaries.

use super::Error;
use is_executable::IsExecutable;
use std::path::PathBuf;

#[cfg(test)]
thread_local! {
	static TMP_DIR: std::cell::RefCell<Option<tempfile::TempDir>> = std::cell::RefCell::new(None);
}

/// Override the workers polkadot binary directory path, used for testing.
#[cfg(test)]
fn workers_exe_path_override() -> Option<PathBuf> {
	TMP_DIR.with_borrow(|t| t.as_ref().map(|t| t.path().join("usr/bin")))
}

/// Override the workers lib directory path, used for testing.
#[cfg(test)]
fn workers_lib_path_override() -> Option<PathBuf> {
	TMP_DIR.with_borrow(|t| t.as_ref().map(|t| t.path().join("usr/lib/polkadot")))
}

/// Determines the final set of paths to use for the PVF workers.
///
/// 1. Get the binaries from the workers path if it is passed in, or consider all possible
/// locations on the filesystem in order and get all sets of paths at which the binaries exist.
///
/// 2. If no paths exist, error out. We can't proceed without workers.
///
/// 3. Log a warning if more than one set of paths exists. Continue with the first set of paths.
///
/// 4. Check if the returned paths are executable. If not it's evidence of a borked installation
/// so error out.
///
/// 5. Do the version check, if mismatch error out.
///
/// 6. At this point the final set of paths should be good to use.
pub fn determine_workers_paths(
	given_workers_path: Option<PathBuf>,
	workers_names: Option<(String, String)>,
	node_version: Option<String>,
) -> Result<(PathBuf, PathBuf), Error> {
	let mut workers_paths = list_workers_paths(given_workers_path.clone(), workers_names.clone())?;
	if workers_paths.is_empty() {
		let current_exe_path = get_exe_path()?;
		return Err(Error::MissingWorkerBinaries {
			given_workers_path,
			current_exe_path,
			workers_names,
		})
	} else if workers_paths.len() > 1 {
		log::warn!("multiple sets of worker binaries found ({:?})", workers_paths,);
	}

	let (prep_worker_path, exec_worker_path) = workers_paths.swap_remove(0);
	if !prep_worker_path.is_executable() || !exec_worker_path.is_executable() {
		return Err(Error::InvalidWorkerBinaries { prep_worker_path, exec_worker_path })
	}

	// Do the version check.
	if let Some(node_version) = node_version {
		let worker_version = polkadot_node_core_pvf::get_worker_version(&prep_worker_path)?;
		if worker_version != node_version {
			return Err(Error::WorkerBinaryVersionMismatch {
				worker_version,
				node_version,
				worker_path: prep_worker_path,
			})
		}

		let worker_version = polkadot_node_core_pvf::get_worker_version(&exec_worker_path)?;
		if worker_version != node_version {
			return Err(Error::WorkerBinaryVersionMismatch {
				worker_version,
				node_version,
				worker_path: exec_worker_path,
			})
		}
	} else {
		log::warn!("Skipping node/worker version checks. This could result in incorrect behavior in PVF workers.");
	}

	Ok((prep_worker_path, exec_worker_path))
}

/// Get list of workers paths by considering the passed-in `given_workers_path` option, or possible
/// locations on the filesystem. See `new_full`.
fn list_workers_paths(
	given_workers_path: Option<PathBuf>,
	workers_names: Option<(String, String)>,
) -> Result<Vec<(PathBuf, PathBuf)>, Error> {
	if let Some(path) = given_workers_path {
		log::trace!("Using explicitly provided workers path {:?}", path);

		if path.is_executable() {
			return Ok(vec![(path.clone(), path)])
		}

		let (prep_worker, exec_worker) = build_worker_paths(path, workers_names);

		// Check if both workers exist. Otherwise return an empty vector which results in an error.
		return if prep_worker.exists() && exec_worker.exists() {
			Ok(vec![(prep_worker, exec_worker)])
		} else {
			Ok(vec![])
		}
	}

	// Workers path not provided, check all possible valid locations.

	let mut workers_paths = vec![];

	// Consider the polkadot binary directory.
	{
		let exe_path = get_exe_path()?;

		let (prep_worker, exec_worker) =
			build_worker_paths(exe_path.clone(), workers_names.clone());

		// Add to set if both workers exist. Warn on partial installs.
		let (prep_worker_exists, exec_worker_exists) = (prep_worker.exists(), exec_worker.exists());
		if prep_worker_exists && exec_worker_exists {
			log::trace!("Worker binaries found at current exe path: {:?}", exe_path);
			workers_paths.push((prep_worker, exec_worker));
		} else if prep_worker_exists {
			log::warn!("Worker binary found at {:?} but not {:?}", prep_worker, exec_worker);
		} else if exec_worker_exists {
			log::warn!("Worker binary found at {:?} but not {:?}", exec_worker, prep_worker);
		}
	}

	// Consider the /usr/lib/polkadot/ directory.
	{
		let lib_path = PathBuf::from("/usr/lib/polkadot");
		#[cfg(test)]
		let lib_path = if let Some(o) = workers_lib_path_override() { o } else { lib_path };

		let (prep_worker, exec_worker) = build_worker_paths(lib_path, workers_names);

		// Add to set if both workers exist. Warn on partial installs.
		let (prep_worker_exists, exec_worker_exists) = (prep_worker.exists(), exec_worker.exists());
		if prep_worker_exists && exec_worker_exists {
			log::trace!("Worker binaries found at /usr/lib/polkadot");
			workers_paths.push((prep_worker, exec_worker));
		} else if prep_worker_exists {
			log::warn!("Worker binary found at {:?} but not {:?}", prep_worker, exec_worker);
		} else if exec_worker_exists {
			log::warn!("Worker binary found at {:?} but not {:?}", exec_worker, prep_worker);
		}
	}

	Ok(workers_paths)
}

fn get_exe_path() -> Result<PathBuf, Error> {
	let mut exe_path = std::env::current_exe()?;
	let _ = exe_path.pop(); // executable file will always have a parent directory.
	#[cfg(test)]
	if let Some(o) = workers_exe_path_override() {
		exe_path = o;
	}

	Ok(exe_path)
}

fn build_worker_paths(
	worker_dir: PathBuf,
	workers_names: Option<(String, String)>,
) -> (PathBuf, PathBuf) {
	let (prep_worker_name, exec_worker_name) = workers_names.unwrap_or((
		polkadot_node_core_pvf::PREPARE_BINARY_NAME.to_string(),
		polkadot_node_core_pvf::EXECUTE_BINARY_NAME.to_string(),
	));

	let mut prep_worker = worker_dir.clone();
	prep_worker.push(prep_worker_name);
	let mut exec_worker = worker_dir;
	exec_worker.push(exec_worker_name);

	(prep_worker, exec_worker)
}

// Tests that set up a temporary directory tree according to what scenario we want to test and
// run worker detection.
#[cfg(test)]
mod tests {
	use super::*;

	use assert_matches::assert_matches;
	use std::{fs, os::unix::fs::PermissionsExt, path::Path};

	const TEST_NODE_VERSION: &'static str = "v0.1.2";

	/// Write a dummy executable to the path which satisfies the version check.
	fn write_worker_exe(path: impl AsRef<Path>) -> Result<(), Box<dyn std::error::Error>> {
		let program = get_program(TEST_NODE_VERSION);
		fs::write(&path, program)?;
		Ok(fs::set_permissions(&path, fs::Permissions::from_mode(0o744))?)
	}

	fn write_worker_exe_invalid_version(
		path: impl AsRef<Path>,
		version: &str,
	) -> Result<(), Box<dyn std::error::Error>> {
		let program = get_program(version);
		fs::write(&path, program)?;
		Ok(fs::set_permissions(&path, fs::Permissions::from_mode(0o744))?)
	}

	fn get_program(version: &str) -> String {
		format!(
			"#!/usr/bin/env bash

if [[ $# -ne 1 ]] ; then
    echo \"unexpected number of arguments: $#\"
    exit 1
fi

if [[ \"$1\" != \"--version\" ]] ; then
    echo \"unexpected argument: $1\"
    exit 1
fi

echo {}
",
			version
		)
	}

	/// Sets up an empty temp dir structure where the workers can be put by tests. Uses the temp dir
	/// to override the standard locations where the node searches for the workers.
	fn with_temp_dir_structure(
		f: impl FnOnce(PathBuf, PathBuf) -> Result<(), Box<dyn std::error::Error>>,
	) -> Result<(), Box<dyn std::error::Error>> {
		// Set up <tmp>/usr/lib/polkadot and <tmp>/usr/bin, both empty.

		let tempdir = tempfile::tempdir().unwrap();
		let tmp_dir = tempdir.path().to_path_buf();
		TMP_DIR.with_borrow_mut(|t| *t = Some(tempdir));

		fs::create_dir_all(workers_lib_path_override().unwrap()).unwrap();
		fs::create_dir_all(workers_exe_path_override().unwrap()).unwrap();

		let custom_path = tmp_dir.join("usr/local/bin");
		// Set up custom path at <tmp>/usr/local/bin.
		fs::create_dir_all(&custom_path).unwrap();

		f(tmp_dir, workers_exe_path_override().unwrap())
	}

	#[test]
	fn test_given_worker_path() {
		with_temp_dir_structure(|tempdir, exe_path| {
			let given_workers_path = tempdir.join("usr/local/bin");

			// Try with provided workers path that has missing binaries.
			assert_matches!(
				determine_workers_paths(Some(given_workers_path.clone()), None, Some(TEST_NODE_VERSION.into())),
				Err(Error::MissingWorkerBinaries { given_workers_path: Some(p1), current_exe_path: p2, workers_names: None }) if p1 == given_workers_path && p2 == exe_path
			);

			// Try with provided workers path that has non-executable binaries.
			let prepare_worker_path = given_workers_path.join("polkadot-prepare-worker");
			write_worker_exe(&prepare_worker_path)?;
			fs::set_permissions(&prepare_worker_path, fs::Permissions::from_mode(0o644))?;
			let execute_worker_path = given_workers_path.join("polkadot-execute-worker");
			write_worker_exe(&execute_worker_path)?;
			fs::set_permissions(&execute_worker_path, fs::Permissions::from_mode(0o644))?;
			assert_matches!(
				determine_workers_paths(Some(given_workers_path.clone()), None, Some(TEST_NODE_VERSION.into())),
				Err(Error::InvalidWorkerBinaries { prep_worker_path: p1, exec_worker_path: p2 }) if p1 == prepare_worker_path && p2 == execute_worker_path
			);

			// Try with valid workers directory path that has executable binaries.
			fs::set_permissions(&prepare_worker_path, fs::Permissions::from_mode(0o744))?;
			fs::set_permissions(&execute_worker_path, fs::Permissions::from_mode(0o744))?;
			assert_matches!(
				determine_workers_paths(Some(given_workers_path), None, Some(TEST_NODE_VERSION.into())),
				Ok((p1, p2)) if p1 == prepare_worker_path && p2 == execute_worker_path
			);

			// Try with valid provided workers path that is a binary file.
			let given_workers_path = tempdir.join("usr/local/bin/test-worker");
			write_worker_exe(&given_workers_path)?;
			assert_matches!(
				determine_workers_paths(Some(given_workers_path.clone()), None, Some(TEST_NODE_VERSION.into())),
				Ok((p1, p2)) if p1 == given_workers_path && p2 == given_workers_path
			);

			Ok(())
		})
		.unwrap();
	}

	#[test]
	fn missing_workers_paths_throws_error() {
		with_temp_dir_structure(|tempdir, exe_path| {
			// Try with both binaries missing.
			assert_matches!(
				determine_workers_paths(None, None, Some(TEST_NODE_VERSION.into())),
				Err(Error::MissingWorkerBinaries { given_workers_path: None, current_exe_path: p, workers_names: None }) if p == exe_path
			);

			// Try with only prep worker (at bin location).
			let prepare_worker_path = tempdir.join("usr/bin/polkadot-prepare-worker");
			write_worker_exe(&prepare_worker_path)?;
			assert_matches!(
				determine_workers_paths(None, None, Some(TEST_NODE_VERSION.into())),
				Err(Error::MissingWorkerBinaries { given_workers_path: None, current_exe_path: p, workers_names: None }) if p == exe_path
			);

			// Try with only exec worker (at bin location).
			fs::remove_file(&prepare_worker_path)?;
			let execute_worker_path = tempdir.join("usr/bin/polkadot-execute-worker");
			write_worker_exe(&execute_worker_path)?;
			assert_matches!(
				determine_workers_paths(None, None, Some(TEST_NODE_VERSION.into())),
				Err(Error::MissingWorkerBinaries { given_workers_path: None, current_exe_path: p, workers_names: None }) if p == exe_path
			);

			// Try with only prep worker (at lib location).
			fs::remove_file(&execute_worker_path)?;
			let prepare_worker_path = tempdir.join("usr/lib/polkadot/polkadot-prepare-worker");
			write_worker_exe(&prepare_worker_path)?;
			assert_matches!(
				determine_workers_paths(None, None, Some(TEST_NODE_VERSION.into())),
				Err(Error::MissingWorkerBinaries { given_workers_path: None, current_exe_path: p, workers_names: None }) if p == exe_path
			);

			// Try with only exec worker (at lib location).
			fs::remove_file(&prepare_worker_path)?;
			let execute_worker_path = tempdir.join("usr/lib/polkadot/polkadot-execute-worker");
			write_worker_exe(execute_worker_path)?;
			assert_matches!(
				determine_workers_paths(None, None, Some(TEST_NODE_VERSION.into())),
				Err(Error::MissingWorkerBinaries { given_workers_path: None, current_exe_path: p, workers_names: None }) if p == exe_path
			);

			Ok(())
		})
		.unwrap()
	}

	#[test]
	fn should_find_workers_at_all_locations() {
		with_temp_dir_structure(|tempdir, _| {
				let prepare_worker_bin_path = tempdir.join("usr/bin/polkadot-prepare-worker");
				write_worker_exe(&prepare_worker_bin_path)?;

				let execute_worker_bin_path = tempdir.join("usr/bin/polkadot-execute-worker");
				write_worker_exe(&execute_worker_bin_path)?;

				let prepare_worker_lib_path = tempdir.join("usr/lib/polkadot/polkadot-prepare-worker");
				write_worker_exe(&prepare_worker_lib_path)?;

				let execute_worker_lib_path = tempdir.join("usr/lib/polkadot/polkadot-execute-worker");
				write_worker_exe(&execute_worker_lib_path)?;

				assert_matches!(
					list_workers_paths(None, None),
					Ok(v) if v == vec![(prepare_worker_bin_path, execute_worker_bin_path), (prepare_worker_lib_path, execute_worker_lib_path)]
				);

				Ok(())
			})
			.unwrap();
	}

	#[test]
	fn should_find_workers_with_custom_names_at_all_locations() {
		with_temp_dir_structure(|tempdir, _| {
			let (prep_worker_name, exec_worker_name) = ("test-prepare", "test-execute");

			let prepare_worker_bin_path = tempdir.join("usr/bin").join(prep_worker_name);
			write_worker_exe(&prepare_worker_bin_path)?;

			let execute_worker_bin_path = tempdir.join("usr/bin").join(exec_worker_name);
			write_worker_exe(&execute_worker_bin_path)?;

			let prepare_worker_lib_path = tempdir.join("usr/lib/polkadot").join(prep_worker_name);
			write_worker_exe(&prepare_worker_lib_path)?;

			let execute_worker_lib_path = tempdir.join("usr/lib/polkadot").join(exec_worker_name);
			write_worker_exe(&execute_worker_lib_path)?;

			assert_matches!(
				list_workers_paths(None, Some((prep_worker_name.into(), exec_worker_name.into()))),
				Ok(v) if v == vec![(prepare_worker_bin_path, execute_worker_bin_path), (prepare_worker_lib_path, execute_worker_lib_path)]
			);

			Ok(())
		})
			.unwrap();
	}

	#[test]
	fn workers_version_mismatch_throws_error() {
		let bad_version = "v9.9.9.9";

		with_temp_dir_structure(|tempdir, _| {
			// Workers at bin location return bad version.
			let prepare_worker_bin_path = tempdir.join("usr/bin/polkadot-prepare-worker");
			let execute_worker_bin_path = tempdir.join("usr/bin/polkadot-execute-worker");
			write_worker_exe_invalid_version(&prepare_worker_bin_path, bad_version)?;
			write_worker_exe(&execute_worker_bin_path)?;
			assert_matches!(
				determine_workers_paths(None, None, Some(TEST_NODE_VERSION.into())),
				Err(Error::WorkerBinaryVersionMismatch { worker_version: v1, node_version: v2, worker_path: p }) if v1 == bad_version && v2 == TEST_NODE_VERSION && p == prepare_worker_bin_path
			);

			// Workers at lib location return bad version.
			fs::remove_file(prepare_worker_bin_path)?;
			fs::remove_file(execute_worker_bin_path)?;
			let prepare_worker_lib_path = tempdir.join("usr/lib/polkadot/polkadot-prepare-worker");
			let execute_worker_lib_path = tempdir.join("usr/lib/polkadot/polkadot-execute-worker");
			write_worker_exe(&prepare_worker_lib_path)?;
			write_worker_exe_invalid_version(&execute_worker_lib_path, bad_version)?;
			assert_matches!(
				determine_workers_paths(None, None, Some(TEST_NODE_VERSION.into())),
				Err(Error::WorkerBinaryVersionMismatch { worker_version: v1, node_version: v2, worker_path: p }) if v1 == bad_version && v2 == TEST_NODE_VERSION && p == execute_worker_lib_path
			);

			// Workers at provided workers location return bad version.
			let given_workers_path = tempdir.join("usr/local/bin");
			let prepare_worker_path = given_workers_path.join("polkadot-prepare-worker");
			let execute_worker_path = given_workers_path.join("polkadot-execute-worker");
			write_worker_exe_invalid_version(&prepare_worker_path, bad_version)?;
			write_worker_exe_invalid_version(&execute_worker_path, bad_version)?;
			assert_matches!(
				determine_workers_paths(Some(given_workers_path), None, Some(TEST_NODE_VERSION.into())),
				Err(Error::WorkerBinaryVersionMismatch { worker_version: v1, node_version: v2, worker_path: p }) if v1 == bad_version && v2 == TEST_NODE_VERSION && p == prepare_worker_path
			);

			// Given worker binary returns bad version.
			let given_workers_path = tempdir.join("usr/local/bin/test-worker");
			write_worker_exe_invalid_version(&given_workers_path, bad_version)?;
			assert_matches!(
				determine_workers_paths(Some(given_workers_path.clone()), None, Some(TEST_NODE_VERSION.into())),
				Err(Error::WorkerBinaryVersionMismatch { worker_version: v1, node_version: v2, worker_path: p }) if v1 == bad_version && v2 == TEST_NODE_VERSION && p == given_workers_path
			);

			Ok(())
		})
		.unwrap();
	}

	#[test]
	fn should_find_valid_workers() {
		// Test bin location.
		with_temp_dir_structure(|tempdir, _| {
			let prepare_worker_bin_path = tempdir.join("usr/bin/polkadot-prepare-worker");
			write_worker_exe(&prepare_worker_bin_path)?;

			let execute_worker_bin_path = tempdir.join("usr/bin/polkadot-execute-worker");
			write_worker_exe(&execute_worker_bin_path)?;

			assert_matches!(
				determine_workers_paths(None, None, Some(TEST_NODE_VERSION.into())),
				Ok((p1, p2)) if p1 == prepare_worker_bin_path && p2 == execute_worker_bin_path
			);

			Ok(())
		})
		.unwrap();

		// Test lib location.
		with_temp_dir_structure(|tempdir, _| {
			let prepare_worker_lib_path = tempdir.join("usr/lib/polkadot/polkadot-prepare-worker");
			write_worker_exe(&prepare_worker_lib_path)?;

			let execute_worker_lib_path = tempdir.join("usr/lib/polkadot/polkadot-execute-worker");
			write_worker_exe(&execute_worker_lib_path)?;

			assert_matches!(
				determine_workers_paths(None, None, Some(TEST_NODE_VERSION.into())),
				Ok((p1, p2)) if p1 == prepare_worker_lib_path && p2 == execute_worker_lib_path
			);

			Ok(())
		})
		.unwrap();
	}
}
