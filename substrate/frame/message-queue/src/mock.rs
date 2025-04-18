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

//! Test helpers and runtime setup for the message queue pallet.

#![cfg(test)]

pub use super::mock_helpers::*;
use super::*;

use crate as pallet_message_queue;
use alloc::collections::btree_map::BTreeMap;
use frame_support::{derive_impl, parameter_types};
use sp_runtime::BuildStorage;

type Block = frame_system::mocking::MockBlock<Test>;

frame_support::construct_runtime!(
	pub enum Test
	{
		System: frame_system,
		MessageQueue: pallet_message_queue,
	}
);

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Block = Block;
}
parameter_types! {
	pub const HeapSize: u32 = 40;
	pub const MaxStale: u32 = 2;
	pub const ServiceWeight: Option<Weight> = Some(Weight::from_parts(100, 100));
}

impl Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = MockedWeightInfo;
	type MessageProcessor = RecordingMessageProcessor;
	type Size = u32;
	type QueueChangeHandler = RecordingQueueChangeHandler;
	type QueuePausedQuery = MockedQueuePauser;
	type HeapSize = HeapSize;
	type MaxStale = MaxStale;
	type ServiceWeight = ServiceWeight;
	type IdleMaxServiceWeight = ServiceWeight;
}

/// Mocked `WeightInfo` impl with allows to set the weight per call.
pub struct MockedWeightInfo;

parameter_types! {
	/// Storage for `MockedWeightInfo`, do not use directly.
	pub static WeightForCall: BTreeMap<String, Weight> = Default::default();
	pub static DefaultWeightForCall: Weight = Weight::zero();
}

/// Set the return value for a function from the `WeightInfo` trait.
impl MockedWeightInfo {
	/// Set the weight of a specific weight function.
	pub fn set_weight<T: Config>(call_name: &str, weight: Weight) {
		let mut calls = WeightForCall::get();
		calls.insert(call_name.into(), weight);
		WeightForCall::set(calls);
	}
}

impl crate::weights::WeightInfo for MockedWeightInfo {
	fn reap_page() -> Weight {
		WeightForCall::get()
			.get("reap_page")
			.copied()
			.unwrap_or(DefaultWeightForCall::get())
	}
	fn execute_overweight_page_updated() -> Weight {
		WeightForCall::get()
			.get("execute_overweight_page_updated")
			.copied()
			.unwrap_or(DefaultWeightForCall::get())
	}
	fn execute_overweight_page_removed() -> Weight {
		WeightForCall::get()
			.get("execute_overweight_page_removed")
			.copied()
			.unwrap_or(DefaultWeightForCall::get())
	}
	fn service_page_base_completion() -> Weight {
		WeightForCall::get()
			.get("service_page_base_completion")
			.copied()
			.unwrap_or(DefaultWeightForCall::get())
	}
	fn service_page_base_no_completion() -> Weight {
		WeightForCall::get()
			.get("service_page_base_no_completion")
			.copied()
			.unwrap_or(DefaultWeightForCall::get())
	}
	fn service_queue_base() -> Weight {
		WeightForCall::get()
			.get("service_queue_base")
			.copied()
			.unwrap_or(DefaultWeightForCall::get())
	}
	fn bump_service_head() -> Weight {
		WeightForCall::get()
			.get("bump_service_head")
			.copied()
			.unwrap_or(DefaultWeightForCall::get())
	}
	fn set_service_head() -> Weight {
		WeightForCall::get()
			.get("set_service_head")
			.copied()
			.unwrap_or(DefaultWeightForCall::get())
	}
	fn service_page_item() -> Weight {
		WeightForCall::get()
			.get("service_page_item")
			.copied()
			.unwrap_or(DefaultWeightForCall::get())
	}
	fn ready_ring_knit() -> Weight {
		WeightForCall::get()
			.get("ready_ring_knit")
			.copied()
			.unwrap_or(DefaultWeightForCall::get())
	}
	fn ready_ring_unknit() -> Weight {
		WeightForCall::get()
			.get("ready_ring_unknit")
			.copied()
			.unwrap_or(DefaultWeightForCall::get())
	}
}

parameter_types! {
	pub static MessagesProcessed: Vec<(Vec<u8>, MessageOrigin)> = vec![];
	/// Queues that should return `Yield` upon being processed.
	pub static YieldingQueues: Vec<MessageOrigin> = vec![];
}

/// A message processor which records all processed messages into [`MessagesProcessed`].
pub struct RecordingMessageProcessor;
impl ProcessMessage for RecordingMessageProcessor {
	/// The transport from where a message originates.
	type Origin = MessageOrigin;

	/// Process the given message, using no more than `weight_limit` in weight to do so.
	///
	/// Consumes exactly `n` weight of all components if it starts `weight=n` and `1` otherwise.
	/// Errors if given the `weight_limit` is insufficient to process the message or if the message
	/// is `badformat`, `corrupt` or `unsupported` with the respective error.
	fn process_message(
		message: &[u8],
		origin: Self::Origin,
		meter: &mut WeightMeter,
		_id: &mut [u8; 32],
	) -> Result<bool, ProcessMessageError> {
		processing_message(message, &origin)?;

		let weight = if message.starts_with(&b"weight="[..]) {
			let mut w: u64 = 0;
			for &c in &message[7..] {
				if (b'0'..=b'9').contains(&c) {
					w = w * 10 + (c - b'0') as u64;
				} else {
					break
				}
			}
			w
		} else {
			1
		};
		let required = Weight::from_parts(weight, weight);

		if meter.try_consume(required).is_ok() {
			if let Some(p) = message.strip_prefix(&b"callback="[..]) {
				let s = String::from_utf8(p.to_vec()).expect("Need valid UTF8");
				if let Err(()) = Callback::get()(&origin, s.parse().expect("Expected an u32")) {
					return Err(ProcessMessageError::Corrupt)
				}

				if s.contains("000") {
					return Ok(false)
				}
			}

			let mut m = MessagesProcessed::get();
			m.push((message.to_vec(), origin));
			MessagesProcessed::set(m);
			Ok(true)
		} else {
			Err(ProcessMessageError::Overweight(required))
		}
	}
}

parameter_types! {
	pub static Callback: Box<fn (&MessageOrigin, u32) -> Result<(), ()>> = Box::new(|_, _| { Ok(()) });
	pub static IgnoreStackOvError: bool = false;
}

/// Processed a mocked message. Messages that end with `badformat`, `corrupt`, `unsupported` or
/// `yield` will fail with an error respectively.
fn processing_message(msg: &[u8], origin: &MessageOrigin) -> Result<(), ProcessMessageError> {
	if YieldingQueues::get().contains(&origin) {
		return Err(ProcessMessageError::Yield)
	}

	let msg = String::from_utf8_lossy(msg);
	if msg.ends_with("badformat") {
		Err(ProcessMessageError::BadFormat)
	} else if msg.ends_with("corrupt") {
		Err(ProcessMessageError::Corrupt)
	} else if msg.ends_with("unsupported") {
		Err(ProcessMessageError::Unsupported)
	} else if msg.ends_with("yield") {
		Err(ProcessMessageError::Yield)
	} else if msg.ends_with("stacklimitreached") && !IgnoreStackOvError::get() {
		Err(ProcessMessageError::StackLimitReached)
	} else {
		Ok(())
	}
}

parameter_types! {
	pub static NumMessagesProcessed: usize = 0;
	pub static NumMessagesErrored: usize = 0;
}

/// Similar to [`RecordingMessageProcessor`] but only counts the number of messages processed and
/// does always consume one weight per message.
///
/// The [`RecordingMessageProcessor`] is a bit too slow for the integration tests.
pub struct CountingMessageProcessor;
impl ProcessMessage for CountingMessageProcessor {
	type Origin = MessageOrigin;

	fn process_message(
		message: &[u8],
		origin: Self::Origin,
		meter: &mut WeightMeter,
		_id: &mut [u8; 32],
	) -> Result<bool, ProcessMessageError> {
		if let Err(e) = processing_message(message, &origin) {
			NumMessagesErrored::set(NumMessagesErrored::get() + 1);
			return Err(e)
		}
		let required = Weight::from_parts(1, 1);

		if meter.try_consume(required).is_ok() {
			if let Some(p) = message.strip_prefix(&b"callback="[..]) {
				let s = String::from_utf8(p.to_vec()).expect("Need valid UTF8");
				if let Err(()) = Callback::get()(&origin, s.parse().expect("Expected an u32")) {
					return Err(ProcessMessageError::Corrupt)
				}
			}

			NumMessagesProcessed::set(NumMessagesProcessed::get() + 1);
			Ok(true)
		} else {
			Err(ProcessMessageError::Overweight(required))
		}
	}
}

parameter_types! {
	/// Storage for `RecordingQueueChangeHandler`, do not use directly.
	pub static QueueChanges: Vec<(MessageOrigin, u64, u64)> = vec![];
}

/// Records all queue changes into [`QueueChanges`].
pub struct RecordingQueueChangeHandler;
impl OnQueueChanged<MessageOrigin> for RecordingQueueChangeHandler {
	fn on_queue_changed(id: MessageOrigin, fp: QueueFootprint) {
		QueueChanges::mutate(|cs| cs.push((id, fp.storage.count, fp.storage.size)));
	}
}

parameter_types! {
	pub static PausedQueues: Vec<MessageOrigin> = vec![];
}

pub struct MockedQueuePauser;
impl QueuePausedQuery<MessageOrigin> for MockedQueuePauser {
	fn is_paused(id: &MessageOrigin) -> bool {
		PausedQueues::get().contains(id)
	}
}

/// Create new test externalities.
///
/// Is generic since it is used by the unit test, integration tests and benchmarks.
pub fn new_test_ext<T: Config>() -> sp_io::TestExternalities
where
	frame_system::pallet_prelude::BlockNumberFor<T>: From<u32>,
{
	sp_tracing::try_init_simple();
	WeightForCall::take();
	QueueChanges::take();
	NumMessagesErrored::take();
	let t = frame_system::GenesisConfig::<T>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| frame_system::Pallet::<T>::set_block_number(1.into()));
	ext
}

/// Run the function pointer inside externalities and asserts the try_state hook at the end.
pub fn build_and_execute<T: Config>(test: impl FnOnce() -> ())
where
	BlockNumberFor<T>: From<u32>,
{
	new_test_ext::<T>().execute_with(|| {
		test();
		pallet_message_queue::Pallet::<T>::do_try_state()
			.expect("All invariants must hold after a test");
	});
}

/// Set the weight of a specific weight function.
pub fn set_weight(name: &str, w: Weight) {
	MockedWeightInfo::set_weight::<Test>(name, w);
}

/// Assert that exactly these pages are present. Assumes `Here` origin.
pub fn assert_pages(indices: &[u32]) {
	assert_eq!(
		Pages::<Test>::iter_keys().count(),
		indices.len(),
		"Wrong number of pages in the queue"
	);
	for i in indices {
		assert!(Pages::<Test>::contains_key(MessageOrigin::Here, i));
	}
}

/// Build a ring with three queues: `Here`, `There` and `Everywhere(0)`.
pub fn build_triple_ring() {
	use MessageOrigin::*;
	build_ring::<Test>(&[Here, There, Everywhere(0)])
}

/// Shim to get rid of the annoying `::<Test>` everywhere.
pub fn assert_ring(queues: &[MessageOrigin]) {
	super::mock_helpers::assert_ring::<Test>(queues);
}

pub fn knit(queue: &MessageOrigin) {
	super::mock_helpers::knit::<Test>(queue);
}

pub fn unknit(queue: &MessageOrigin) {
	super::mock_helpers::unknit::<Test>(queue);
}

pub fn num_overweight_enqueued_events() -> u32 {
	frame_system::Pallet::<Test>::events()
		.into_iter()
		.filter(|e| {
			matches!(e.event, RuntimeEvent::MessageQueue(crate::Event::OverweightEnqueued { .. }))
		})
		.count() as u32
}

pub fn fp(pages: u32, ready_pages: u32, count: u64, size: u64) -> QueueFootprint {
	QueueFootprint { storage: Footprint { count, size }, pages, ready_pages }
}

/// A random seed that can be overwritten with `MQ_SEED`.
pub fn gen_seed() -> u64 {
	use rand::Rng;
	let seed = if let Ok(seed) = std::env::var("MQ_SEED") {
		seed.parse().expect("Need valid u64 as MQ_SEED env variable")
	} else {
		rand::thread_rng().gen::<u64>()
	};

	println!("Using seed: {}", seed);
	seed
}
