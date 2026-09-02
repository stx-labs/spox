//! Poller for detecting new blocks on the Bitcoin blockchain.
//!
//! [`BitcoinChainTipPoller`] periodically calls [`BitcoinChainTipCaller::get_chain_tip`]
//! (for Bitcoin Core, the active tip from `getchaintips`). When it detects a new
//! tip, it broadcasts that [`BlockRef`] to all subscribers.
//!

use std::time::Duration;

use tokio::sync::broadcast;

use crate::bitcoin::BlockRef;
use crate::bitcoin::node::BitcoinCoreClient;
use crate::error::Error;

/// The default capacity of the broadcast channel for sending new block hashes.
const CHAIN_TIP_CHANNEL_CAPACITY: usize = 144;

/// A trait for fetching the current Bitcoin chain tip.
pub trait BitcoinChainTipCaller: Send + Sync {
    /// Get the current Bitcoin chain tip.
    fn get_chain_tip(&self) -> Result<BlockRef, Error>;
}

impl BitcoinChainTipCaller for BitcoinCoreClient {
    fn get_chain_tip(&self) -> Result<BlockRef, Error> {
        self.get_chain_tip()
    }
}

/// A poller that periodically checks for and broadcasts new Bitcoin chain tips.
///
/// This struct manages a background task that polls a Bitcoin Core node's RPC
/// to get the latest chain tip. It provides a stream of these tips that other
/// parts of the application can subscribe to.
#[derive(Debug)]
pub struct BitcoinChainTipPoller {
    /// The sender for the broadcast channel that distributes new chain tips.
    sender: broadcast::Sender<BlockRef>,
}

/// Runs the RPC polling loop in a background task.
///
/// This function polls [`BitcoinChainTipCaller::get_chain_tip`] at a regular
/// interval, detects new tips, and broadcasts them on the provided channel.
async fn run_poller<B>(rpc: B, sender: broadcast::Sender<BlockRef>, polling_interval: Duration)
where
    B: BitcoinChainTipCaller,
{
    let mut last_seen: Option<BlockRef> = None;

    loop {
        match rpc.get_chain_tip() {
            Ok(tip) if Some(&tip) != last_seen.as_ref() => {
                tracing::debug!(%tip, "detected new bitcoin chain tip");
                match sender.send(tip) {
                    Ok(_) => last_seen = Some(tip),
                    Err(_) => tracing::warn!("no active subscribers for chain tip broadcast"),
                }
            }
            Err(error) => {
                tracing::warn!(%error, "failed to get chain tip during polling; will retry");
            }
            _ => {}
        }

        tokio::time::sleep(polling_interval).await;
    }
}

impl BitcoinChainTipPoller {
    /// Creates and starts a new `BitcoinChainTipPoller` task.
    ///
    /// Spawns a background task that polls `rpc` for chain tip changes.
    pub async fn start<B>(rpc: B, polling_interval: Duration) -> Self
    where
        B: BitcoinChainTipCaller + 'static,
    {
        let (sender, _) = broadcast::channel::<BlockRef>(CHAIN_TIP_CHANNEL_CAPACITY);

        // Spawn the RPC polling task.
        tokio::spawn(run_poller(rpc, sender.clone(), polling_interval));

        Self { sender }
    }

    /// Subscribes to the poller, returning a receiver of new chain tips.
    pub fn new_receiver(&self) -> broadcast::Receiver<BlockRef> {
        self.sender.subscribe()
    }
}
