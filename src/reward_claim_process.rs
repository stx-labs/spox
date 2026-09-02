//! Tip-driven process that advances reward claims and settlements.
//!
//! On each new Bitcoin chain tip the process:
//! 1. Fetches pending claims and broadcasts `process-reward-claims` batches,
//! 2. Fetches pending settlements and broadcasts `settle-pending-withdrawals`
//!    batches.
//!
//! Run via [`crate::dispatch::run_on_chain_tips`] alongside deposit
//! monitoring (see `main`).

use tokio::sync::mpsc;

use crate::bitcoin::BlockRef;
use crate::context::Context;
use crate::error::Error;

/// The loop for processing reward claims that runs whenever a new Bitcoin
/// block is detected.
pub async fn process_reward_claims(mut rx: mpsc::Receiver<BlockRef>, context: Context) {
    while let Some(chain_tip) = rx.recv().await {
        if let Err(error) = process_pending_claims(&context, &chain_tip).await {
            tracing::warn!(%error, "error processing pending reward claims");
        }

        if let Err(error) = process_pending_settlements(&context, &chain_tip).await {
            tracing::warn!(%error, "error processing pending settlements");
        }
    }
}

/// The function that processes pending claims.
///
/// # Notes
///
/// This function works as follows:
/// 1. Gets all pending claims from the registry.
/// 2. Submits a process-reward-claims contract call for each batch of
///    claims, where a batch is a group of at most 100 stakers who are
///    associated with the same signer-manager.
async fn process_pending_claims(_: &Context, chain_tip: &BlockRef) -> Result<(), Error> {
    // TODO(#41/#42): fetch pending claims and submit process-reward-claims.
    tracing::debug!(%chain_tip, "reward claim processing not yet implemented");
    Ok(())
}

/// The function that processes pending settlements.
///
/// # Notes
///
/// This function works as follows:
/// 1. Gets all pending settlements from the registry.
/// 2. Submits a settle-pending-withdrawals contract call for each batch of
///    settlements, where a batch is a group of at most 100 stakers who are
///    associated with the same signer-manager.
async fn process_pending_settlements(_: &Context, chain_tip: &BlockRef) -> Result<(), Error> {
    // TODO(#40/#42): fetch pending settlements and submit settle-pending-withdrawals.
    tracing::debug!(%chain_tip, "reward settlement processing not yet implemented");
    Ok(())
}
