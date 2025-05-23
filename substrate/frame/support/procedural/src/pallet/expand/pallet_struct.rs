// This file is part of Substrate.

// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::pallet::{expand::merge_where_clauses, Def};
use frame_support_procedural_tools::get_doc_literals;

///
/// * Add derive trait on Pallet
/// * Implement GetStorageVersion on Pallet
/// * Implement OnGenesis on Pallet
/// * Implement `fn error_metadata` on Pallet
/// * declare Module type alias for construct_runtime
/// * replace the first field type of `struct Pallet` with `PhantomData` if it is `_`
/// * implementation of `PalletInfoAccess` information
/// * implementation of `StorageInfoTrait` on Pallet
pub fn expand_pallet_struct(def: &mut Def) -> proc_macro2::TokenStream {
	let frame_support = &def.frame_support;
	let frame_system = &def.frame_system;
	let type_impl_gen = &def.type_impl_generics(def.pallet_struct.attr_span);
	let type_use_gen = &def.type_use_generics(def.pallet_struct.attr_span);
	let type_decl_gen = &def.type_decl_generics(def.pallet_struct.attr_span);
	let pallet_ident = &def.pallet_struct.pallet;
	let config_where_clause = &def.config.where_clause;
	let deprecation_status =
		match crate::deprecation::get_deprecation(&quote::quote! {#frame_support}, &def.item.attrs)
		{
			Ok(deprecation) => deprecation,
			Err(e) => return e.into_compile_error(),
		};

	let mut storages_where_clauses = vec![&def.config.where_clause];
	storages_where_clauses.extend(def.storages.iter().map(|storage| &storage.where_clause));
	let storages_where_clauses = merge_where_clauses(&storages_where_clauses);

	let pallet_item = {
		let pallet_module_items = &mut def.item.content.as_mut().expect("Checked by def").1;
		let item = &mut pallet_module_items[def.pallet_struct.index];
		if let syn::Item::Struct(item) = item {
			item
		} else {
			unreachable!("Checked by pallet struct parser")
		}
	};

	// If the first field type is `_` then we replace with `PhantomData`
	if let Some(field) = pallet_item.fields.iter_mut().next() {
		if field.ty == syn::parse_quote!(_) {
			field.ty = syn::parse_quote!(
				core::marker::PhantomData<(#type_use_gen)>
			);
		}
	}

	if get_doc_literals(&pallet_item.attrs).is_empty() {
		pallet_item.attrs.push(syn::parse_quote!(
			#[doc = r"
				The `Pallet` struct, the main type that implements traits and standalone
				functions within the pallet.
			"]
		));
	}

	pallet_item.attrs.push(syn::parse_quote!(
		#[derive(
			#frame_support::CloneNoBound,
			#frame_support::EqNoBound,
			#frame_support::PartialEqNoBound,
			#frame_support::RuntimeDebugNoBound,
		)]
	));

	let pallet_error_metadata = if let Some(error_def) = &def.error {
		let error_ident = &error_def.error;
		quote::quote_spanned!(def.pallet_struct.attr_span =>
			impl<#type_impl_gen> #pallet_ident<#type_use_gen> #config_where_clause {
				#[doc(hidden)]
				#[allow(deprecated)]
				pub fn error_metadata() -> Option<#frame_support::__private::metadata_ir::PalletErrorMetadataIR> {
					Some(<#error_ident<#type_use_gen>>::error_metadata())
				}
			}
		)
	} else {
		quote::quote_spanned!(def.pallet_struct.attr_span =>
			impl<#type_impl_gen> #pallet_ident<#type_use_gen> #config_where_clause {
				#[doc(hidden)]
				pub fn error_metadata() -> Option<#frame_support::__private::metadata_ir::PalletErrorMetadataIR> {
					None
				}
			}
		)
	};

	let storage_info_span =
		def.pallet_struct.without_storage_info.unwrap_or(def.pallet_struct.attr_span);

	let storage_names = &def.storages.iter().map(|storage| &storage.ident).collect::<Vec<_>>();
	let storage_cfg_attrs =
		&def.storages.iter().map(|storage| &storage.cfg_attrs).collect::<Vec<_>>();
	let storage_maybe_allow_attrs = &def
		.storages
		.iter()
		.map(|storage| crate::deprecation::extract_or_return_allow_attrs(&storage.attrs).collect())
		.collect::<Vec<Vec<_>>>();
	// Depending on the flag `without_storage_info` and the storage attribute `unbounded`, we use
	// partial or full storage info from storage.
	let storage_info_traits = &def
		.storages
		.iter()
		.map(|storage| {
			if storage.unbounded || def.pallet_struct.without_storage_info.is_some() {
				quote::quote_spanned!(storage_info_span => PartialStorageInfoTrait)
			} else {
				quote::quote_spanned!(storage_info_span => StorageInfoTrait)
			}
		})
		.collect::<Vec<_>>();

	let storage_info_methods = &def
		.storages
		.iter()
		.map(|storage| {
			if storage.unbounded || def.pallet_struct.without_storage_info.is_some() {
				quote::quote_spanned!(storage_info_span => partial_storage_info)
			} else {
				quote::quote_spanned!(storage_info_span => storage_info)
			}
		})
		.collect::<Vec<_>>();

	let storage_info = quote::quote_spanned!(storage_info_span =>
		impl<#type_impl_gen> #frame_support::traits::StorageInfoTrait
			for #pallet_ident<#type_use_gen>
			#storages_where_clauses
		{
			fn storage_info()
				-> #frame_support::__private::Vec<#frame_support::traits::StorageInfo>
			{
				#[allow(unused_mut)]
				let mut res = #frame_support::__private::vec![];

				#(
					#(#storage_cfg_attrs)*
					#(#storage_maybe_allow_attrs)*
					{
						let mut storage_info = <
							#storage_names<#type_use_gen>
							as #frame_support::traits::#storage_info_traits
						>::#storage_info_methods();
						res.append(&mut storage_info);
					}
				)*

				res
			}
		}
	);

	let (storage_version, in_code_storage_version_ty) =
		if let Some(v) = def.pallet_struct.storage_version.as_ref() {
			(quote::quote! { #v }, quote::quote! { #frame_support::traits::StorageVersion })
		} else {
			(
				quote::quote! { core::default::Default::default() },
				quote::quote! { #frame_support::traits::NoStorageVersionSet },
			)
		};

	let whitelisted_storage_idents: Vec<syn::Ident> = def
		.storages
		.iter()
		.filter_map(|s| s.whitelisted.then(|| s.ident.clone()))
		.collect();

	let whitelisted_storage_keys_impl = quote::quote![
		use #frame_support::traits::{StorageInfoTrait, TrackedStorageKey, WhitelistedStorageKeys};
		impl<#type_impl_gen> WhitelistedStorageKeys for #pallet_ident<#type_use_gen> #storages_where_clauses {
			fn whitelisted_storage_keys() -> #frame_support::__private::Vec<TrackedStorageKey> {
				use #frame_support::__private::vec;
				vec![#(
					TrackedStorageKey::new(#whitelisted_storage_idents::<#type_use_gen>::hashed_key().to_vec())
				),*]
			}
		}
	];

	quote::quote_spanned!(def.pallet_struct.attr_span =>
		#pallet_error_metadata

		/// Type alias to `Pallet`, to be used by `construct_runtime`.
		///
		/// Generated by `pallet` attribute macro.
		#[deprecated(note = "use `Pallet` instead")]
		#[allow(dead_code)]
		pub type Module<#type_decl_gen> = #pallet_ident<#type_use_gen>;

		// Implement `GetStorageVersion` for `Pallet`
		impl<#type_impl_gen> #frame_support::traits::GetStorageVersion
			for #pallet_ident<#type_use_gen>
			#config_where_clause
		{
			type InCodeStorageVersion = #in_code_storage_version_ty;

			fn in_code_storage_version() -> Self::InCodeStorageVersion {
				#storage_version
			}

			fn on_chain_storage_version() -> #frame_support::traits::StorageVersion {
				#frame_support::traits::StorageVersion::get::<Self>()
			}
		}

		// Implement `OnGenesis` for `Pallet`
		impl<#type_impl_gen> #frame_support::traits::OnGenesis
			for #pallet_ident<#type_use_gen>
			#config_where_clause
		{
			fn on_genesis() {
				let storage_version: #frame_support::traits::StorageVersion = #storage_version;
				storage_version.put::<Self>();
			}
		}

		// Implement `PalletInfoAccess` for `Pallet`
		impl<#type_impl_gen> #frame_support::traits::PalletInfoAccess
			for #pallet_ident<#type_use_gen>
			#config_where_clause
		{
			fn index() -> usize {
				<
					<T as #frame_system::Config>::PalletInfo as #frame_support::traits::PalletInfo
				>::index::<Self>()
					.expect("Pallet is part of the runtime because pallet `Config` trait is \
						implemented by the runtime")
			}

			fn name() -> &'static str {
				<
					<T as #frame_system::Config>::PalletInfo as #frame_support::traits::PalletInfo
				>::name::<Self>()
					.expect("Pallet is part of the runtime because pallet `Config` trait is \
						implemented by the runtime")
			}

			fn name_hash() -> [u8; 16] {
				<
					<T as #frame_system::Config>::PalletInfo as #frame_support::traits::PalletInfo
				>::name_hash::<Self>()
					.expect("Pallet is part of the runtime because pallet `Config` trait is \
						implemented by the runtime")
			}

			fn module_name() -> &'static str {
				<
					<T as #frame_system::Config>::PalletInfo as #frame_support::traits::PalletInfo
				>::module_name::<Self>()
					.expect("Pallet is part of the runtime because pallet `Config` trait is \
						implemented by the runtime")
			}

			fn crate_version() -> #frame_support::traits::CrateVersion {
				#frame_support::crate_to_crate_version!()
			}
		}

		impl<#type_impl_gen> #frame_support::traits::PalletsInfoAccess
			for #pallet_ident<#type_use_gen>
			#config_where_clause
		{
			fn count() -> usize { 1 }
			fn infos() -> #frame_support::__private::Vec<#frame_support::traits::PalletInfoData> {
				use #frame_support::traits::PalletInfoAccess;
				let item = #frame_support::traits::PalletInfoData {
					index: Self::index(),
					name: Self::name(),
					module_name: Self::module_name(),
					crate_version: Self::crate_version(),
				};
				#frame_support::__private::vec![item]
			}
		}

		#storage_info
		#whitelisted_storage_keys_impl

		impl<#type_use_gen> #pallet_ident<#type_use_gen> {
			#[allow(dead_code)]
			#[doc(hidden)]
			pub fn deprecation_info() -> #frame_support::__private::metadata_ir::DeprecationStatusIR {
				#deprecation_status
			}
		}
	)
}
