#![doc = include_str!("../README.md")]
#![deny(missing_docs)]

use std::num::NonZeroUsize;

pub mod bitcoin;
pub mod config;
pub mod context;
pub mod deposit_monitor;
pub mod dispatch;
pub mod error;
pub mod logging;
pub mod reward_claim_process;
pub mod stacks;
pub mod storage;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

/// The capacity of the channel for sending new Bitcoin chain tips to a
/// process.
///
/// Each process should have their own channel to avoid blocking the main
/// broadcast channel.
pub const MAILBOX_CAPACITY: usize = 144;

/// Default number of deposits submitted to Emily for each new Bitcoin
/// block.
pub const DEFAULT_MAX_DEPOSITS_PER_BLOCK: NonZeroUsize = NonZeroUsize::new(25).unwrap();
