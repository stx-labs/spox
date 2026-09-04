//! Tip-driven process that advances reward claims and withdrawals.
//!
//! On each new Bitcoin chain tip the process:
//! 1. Fetches pending claims and broadcasts `process-reward-claims` batches,
//! 2. Fetches pending withdrawals and broadcasts `settle-pending-withdrawals`
//!    batches.
//!
//! Run via [`crate::dispatch::run_on_chain_tips`] alongside deposit
//! monitoring (see `main`).

use tokio::sync::mpsc;
use tracing::instrument;

use crate::bitcoin::BlockRef;
use crate::config::Settings;
use crate::context::Context;
use crate::error::Error;
use crate::stacks::node::StacksClient;
use crate::stacks::node::SubmitTxResponse;
use crate::stacks::reward_claim_registry::RewardClaimRegistry;
use crate::stacks::transaction::IntoContractCall as _;
use crate::stacks::wallet::StacksWallet;

/// The transaction fee for all contract call transactions against the
/// registry.
/// TODO: remove and fetch the market fee rate from the node.
const TX_FEE: u64 = 100000;

/// The loop for processing reward claims that runs whenever a new Bitcoin
/// block is detected.
pub async fn process_reward_claims(mut rx: mpsc::Receiver<BlockRef>, context: Context) {
    if context.settings().reward_claims.is_none() {
        tracing::info!("reward claims are not configured, skipping");
        return;
    }

    let state = match RewardClaimState::new(context.settings()).await {
        Ok(claims) => claims,
        Err(error) => {
            tracing::error!(%error, "failed to initialize reward claims process");
            return;
        }
    };

    while let Some(chain_tip) = rx.recv().await {
        // We update the wallet nonce before constucting any transactions.
        // There is a risk that we have transactions in the mempool so
        // using the fetched nonce will lead to an RBF attempt. This
        // attempt could fail, because the proposed fee could be too low.
        // However, with ~10 average bitcoin blocks and prompt stacks
        // confirmations, this should rarely be an issue.
        if let Err(error) = state.update_wallet_nonce().await {
            tracing::warn!(%error, "failed to update wallet nonce");
            continue;
        }

        if let Err(error) = process_claims(&state, &chain_tip).await {
            tracing::warn!(%error, "error processing pending reward claims");
        }

        if let Err(error) = process_withdrawals(&state, &chain_tip).await {
            tracing::warn!(%error, "error processing pending withdrawals");
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
#[instrument(skip_all, fields(bitcoin_block_height = %chain_tip.block_height, bitcoin_block_hash = %chain_tip.block_hash))]
async fn process_claims(state: &RewardClaimState, chain_tip: &BlockRef) -> Result<(), Error> {
    let batches = state.registry.get_pending_claim_batches().await?;
    if batches.is_empty() {
        tracing::info!("no pending reward claims");
        return Ok(());
    }

    for batch in batches {
        tracing::info!(
            "signer_manager" = %batch.signer_manager(),
            "num_stakers" = %batch.num_stakers(),
            "processing process-reward-claims batch",
        );
        let payload = batch.into_tx_payload();
        let tx = state.wallet.sign_tx(payload, TX_FEE);

        match state.client().submit_tx(&tx).await {
            Ok(SubmitTxResponse::Acceptance(txid)) => {
                tracing::info!(%txid, "submitted process-reward-claims batch");
                state.increment_wallet_nonce();
            }
            Ok(SubmitTxResponse::Rejection(error)) => {
                tracing::warn!(%error, "failed to submit process-reward-claims batch");
            }
            Err(error) => {
                // It could be the case that we broadcast the transaction
                // to the node and it was rejected by then we got an error
                // here anyway. I don't see a clean way to handle this
                // without adding another issue.
                tracing::warn!(%error, "failed to submit process-reward-claims batch");
            }
        }
    }

    tracing::debug!(%chain_tip, "finished process-reward-claims submissions");
    Ok(())
}

/// The function that processes pending withdrawals.
///
/// # Notes
///
/// This function works as follows:
/// 1. Gets all pending withdrawals from the registry.
/// 2. Submits a settle-pending-withdrawals contract call for each batch of
///    withdrawals, where a batch is a group of at most 100 withdrawals who
///    are associated with the same signer-manager.
#[instrument(skip_all, fields(bitcoin_block_height = %chain_tip.block_height, bitcoin_block_hash = %chain_tip.block_hash))]
async fn process_withdrawals(state: &RewardClaimState, chain_tip: &BlockRef) -> Result<(), Error> {
    let batches = state.registry.get_pending_withdrawal_batches().await?;
    if batches.is_empty() {
        tracing::info!("no pending reward withdrawals");
        return Ok(());
    }

    for batch in batches {
        tracing::info!(
            "signer_manager" = %batch.signer_manager(),
            "num_withdrawals" = %batch.num_withdrawals(),
            "processing settle-pending-withdrawals batch",
        );
        let payload = batch.into_tx_payload();
        let tx = state.wallet.sign_tx(payload, TX_FEE);

        match state.client().submit_tx(&tx).await {
            Ok(SubmitTxResponse::Acceptance(txid)) => {
                tracing::info!(%txid, "submitted settle-pending-withdrawals batch");
                state.increment_wallet_nonce();
            }
            Ok(SubmitTxResponse::Rejection(error)) => {
                tracing::warn!(%error, "failed to submit settle-pending-withdrawals batch");
            }
            Err(error) => {
                // It could be the case that we broadcast the transaction
                // to the node and it was rejected by then we got an error
                // here anyway. I don't see a clean way to handle this
                // without adding another issue.
                tracing::warn!(%error, "failed to submit settle-pending-withdrawals batch");
            }
        }
    }

    tracing::debug!(%chain_tip, "finished settle-pending-withdrawals submissions");
    Ok(())
}

/// The context needed for the reward-claims loop.
#[derive(Debug)]
pub struct RewardClaimState {
    /// The reward-claims registry
    registry: RewardClaimRegistry,
    /// A wallet for signing Stacks transactions
    wallet: StacksWallet,
}

impl RewardClaimState {
    /// Construct a Stacks wallet given the information in the config.
    ///
    /// # Note
    ///
    /// This function reaches out to the stacks node to get the current
    /// chain ID.
    pub async fn new(settings: &Settings) -> Result<Self, Error> {
        let Some(config) = settings.reward_claims.clone() else {
            return Err(Error::MissingRewardClaimsConfig);
        };
        let Some(stacks) = settings.stacks.as_ref() else {
            return Err(Error::MissingStacksConfig);
        };

        // Let's go and get the current chain id.
        let client = StacksClient::new(stacks.rpc_endpoint.clone(), &stacks.auth_token)?;
        let info = client.get_node_info().await?;
        let wallet = StacksWallet::new(config.private_key, info.chain_id, 0);

        let registry = RewardClaimRegistry::new(config.claims_contract, client);

        Ok(Self { registry, wallet })
    }

    /// Update the account nonce for the wallet.
    pub async fn update_wallet_nonce(&self) -> Result<(), Error> {
        let address = self.wallet.address();
        let account = self.registry.client().get_account(address).await?;
        self.wallet.set_nonce(account.nonce);
        Ok(())
    }

    /// Increment the wallet nonce by 1.
    pub fn increment_wallet_nonce(&self) {
        self.wallet.increment_nonce();
    }

    /// Get a reference to the client.
    pub fn client(&self) -> &StacksClient {
        self.registry.client()
    }
}
