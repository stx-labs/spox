//! This module provides a dispatcher for running functions whenever a new
//! Bitcoin chain tip is detected.

use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;

use crate::MAILBOX_CAPACITY;
use crate::bitcoin::BlockRef;
use crate::context::Context;

/// Spawn a process that runs a function whenever a new Bitcoin chain
/// tip is detected.
pub async fn run_on_chain_tips<F, O>(func: F, mut rx: Receiver<BlockRef>, context: Context)
where
    F: Fn(mpsc::Receiver<BlockRef>, Context) -> O + Send + 'static,
    O: Future + Send + 'static,
    O::Output: Send + 'static,
{
    let (sender, mpsc_rx) = mpsc::channel::<BlockRef>(MAILBOX_CAPACITY);

    tokio::spawn(func(mpsc_rx, context));

    loop {
        let chain_tip = match rx.recv().await {
            Ok(chain_tip) => chain_tip,
            Err(RecvError::Lagged(skipped)) => {
                tracing::warn!(%skipped, "lagged behind bitcoin chain tip broadcast");
                continue;
            }
            Err(RecvError::Closed) => {
                tracing::warn!("bitcoin chain tip broadcast closed");
                break;
            }
        };

        if let Err(error) = sender.try_send(chain_tip) {
            tracing::warn!(%error, "error sending new bitcoin chain tip to processor");
        }
    }
}
