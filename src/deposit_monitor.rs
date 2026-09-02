//! Module to monitor for pending deposits

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::str::FromStr as _;

use bitcoin::{BlockHash, ScriptBuf, Txid};
use emily_client::apis::deposit_api;
use emily_client::models::CreateDepositRequestBody;
use lru::LruCache;
use tokio::sync::mpsc;

use crate::bitcoin::wallet::import_descriptors;
use crate::bitcoin::{BlockRef, Utxo};
use crate::context::Context;
use crate::error::Error;
use crate::stacks::registry::GET_ADDRESSES_MAX_IDS;
use crate::storage::Storage as _;

/// Deposit monitor
pub struct DepositMonitor {
    context: Context,
    tx_hex_cache: LruCache<(Txid, BlockHash), String>,
    created_deposits: LruCache<(Txid, u32), ()>,
}

// TODO: make cache size configurable
// As for now numbers are chosen to keep cache size around 4MB
const TX_HEX_CACHE_SIZE: NonZeroUsize =
    NonZeroUsize::new(8_000).expect("Cache size must be non-zero");

/// How many created deposits to keep track of. This should keep the max memory
/// usage below 10MB.
const CREATED_DEPOSITS_CACHE_SIZE: NonZeroUsize =
    NonZeroUsize::new(100_000).expect("Cache size must be non-zero");

impl DepositMonitor {
    /// Creates a new `DepositMonitor`
    pub fn new(context: Context) -> Self {
        Self {
            context,
            tx_hex_cache: LruCache::new(TX_HEX_CACHE_SIZE),
            created_deposits: LruCache::new(CREATED_DEPOSITS_CACHE_SIZE),
        }
    }

    /// Process a `Utxo` to get a create deposit request for Emily
    pub fn get_deposit_from_utxo(
        &mut self,
        utxo: &Utxo,
        chain_tip: &BlockRef,
    ) -> Result<CreateDepositRequestBody, Error> {
        let monitored_deposit = self
            .context
            .storage()
            .get_by_script(&utxo.script_pub_key)?
            .ok_or_else(|| Error::MissingMonitoredDeposit(utxo.script_pub_key.clone()))?;

        let unlocking_time =
            utxo.block_height + (monitored_deposit.reclaim_script_inputs.lock_time() as u64);
        if unlocking_time <= chain_tip.block_height {
            return Err(Error::DepositExpired);
        }

        let bitcoin_client = self.context.bitcoin_client();

        let block_hash = bitcoin_client.get_block_hash(utxo.block_height)?;

        let tx_hex = self
            .tx_hex_cache
            .try_get_or_insert((utxo.txid, block_hash), || {
                bitcoin_client.get_raw_transaction_hex(&utxo.txid, &block_hash)
            })?
            .clone();

        Ok(CreateDepositRequestBody {
            bitcoin_tx_output_index: utxo.vout,
            bitcoin_txid: utxo.txid.to_string(),
            deposit_script: monitored_deposit
                .deposit_script_inputs
                .deposit_script()
                .to_hex_string(),
            reclaim_script: monitored_deposit
                .reclaim_script_inputs
                .reclaim_script()
                .to_hex_string(),
            transaction_hex: tx_hex,
        })
    }

    fn get_utxos(
        &self,
        script_pubkeys: &[ScriptBuf],
        chain_tip: &BlockRef,
    ) -> Result<Vec<Utxo>, Error> {
        let bitcoin = self.context.bitcoin_client();
        if self.context.settings().node_wallet.is_none() {
            if bitcoin.scan_tx_out_set_scanning()? {
                tracing::warn!(
                    "scan already in progress, attempting a new one will fail; skipping fetching utxos"
                );
                return Err(Error::ScanAlreadyInProgress);
            }

            // TODO: batch the scan_tx_out_set call
            return bitcoin
                .scan_tx_out_set(script_pubkeys)
                .inspect_err(|error| {
                    if matches!(error, Error::ScanAlreadyInProgress) {
                        tracing::warn!(
                            "scan didn't finish in time, consider increasing the rpc timeout"
                        )
                    }
                });
        }

        let watched_script_pubkeys: HashSet<&ScriptBuf> = script_pubkeys.iter().collect();

        let utxos = bitcoin
            .list_unspent()?
            .into_iter()
            .filter_map(|unspent| {
                if !watched_script_pubkeys.contains(&unspent.script_pub_key) {
                    return None;
                }
                // Note that if a new block is observed between when we fetched
                // the chain tip and the UTXOs, this computation may be off by
                // one block: this is fine, we will fail to find the transaction
                // in the expected block and try again at the next block.
                // 1 confirmation means the UTXO is confirmed on chain tip.
                let block_height = chain_tip
                    .block_height
                    .saturating_sub(unspent.confirmations.saturating_sub(1) as u64);

                Some(Utxo {
                    txid: unspent.txid,
                    vout: unspent.vout,
                    script_pub_key: unspent.script_pub_key,
                    amount: unspent.amount,
                    block_height,
                })
            })
            .collect();

        Ok(utxos)
    }

    /// Check pending deposits confirmed to the monitored addresses
    pub fn get_pending_deposits(
        &mut self,
        chain_tip: &BlockRef,
    ) -> Result<Vec<CreateDepositRequestBody>, Error> {
        let script_pubkeys = self.context.storage().get_scripts()?;
        if script_pubkeys.is_empty() {
            return Ok(Vec::new());
        }
        let utxos = self.get_utxos(&script_pubkeys, chain_tip)?;

        let create_deposits = utxos
            .iter()
            .filter_map(|utxo| {
                // Emily will nop for duplicates, still we try avoiding wasting
                // time for deposits we already created in this session.
                if self.created_deposits.get(&(utxo.txid, utxo.vout)).is_some() {
                    return None;
                }

                self.get_deposit_from_utxo(utxo, chain_tip)
                    .inspect_err(|error| match error {
                        Error::DepositExpired => tracing::info!(
                            %error,
                            txid = %utxo.txid,
                            vout = %utxo.vout,
                            block_height = %utxo.block_height,
                            "deposit is expired; skipping utxo"
                        ),
                        _ => tracing::warn!(
                            %error,
                            txid = %utxo.txid,
                            vout = %utxo.vout,
                            block_height = %utxo.block_height,
                            "failed to get deposit from utxo; skipping utxo"
                        ),
                    })
                    .ok()
            })
            .collect();

        Ok(create_deposits)
    }

    /// Mark a deposit as (locally) created
    pub fn deposit_created(&mut self, bitcoin_txid: &str, bitcoin_tx_output_index: u32) {
        match Txid::from_str(bitcoin_txid) {
            Ok(txid) => {
                self.created_deposits
                    .put((txid, bitcoin_tx_output_index), ());
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    txid = %bitcoin_txid,
                    vout = %bitcoin_tx_output_index,
                    "failed to parse transaction id"
                );
            }
        };
    }
}

/// The loop for processing monitored deposits whenever a new Bitcoin block
/// is detected.
pub async fn process_monitored_deposits(mut rx: mpsc::Receiver<BlockRef>, context: Context) {
    let mut deposit_monitor = DepositMonitor::new(context.clone());

    while let Some(chain_tip) = rx.recv().await {
        if let Err(error) = update_from_registry(&context).await {
            tracing::warn!(%error, "error updating from address registry");
        }

        if let Err(error) = update_wallet(&context) {
            tracing::warn!(%error, "error updating the wallet");
        }

        if let Err(error) =
            fetch_and_create_deposits(&context, &mut deposit_monitor, &chain_tip).await
        {
            tracing::warn!(%error, "error processing pending deposits");
        }
    }
}

async fn update_from_registry(context: &Context) -> Result<(), Error> {
    let Some(registry) = context.registry() else {
        return Ok(());
    };
    let store = context.storage();

    let last_next_id = store.get_last_next_address_id()?;
    let registry_next_id = registry.get_next_address_id().await?;

    if last_next_id >= registry_next_id {
        return Ok(());
    }

    // TODO: limit the amount of work per single function invocation
    for start in (last_next_id..registry_next_id).step_by(GET_ADDRESSES_MAX_IDS as usize) {
        let end = start
            .saturating_add(GET_ADDRESSES_MAX_IDS as u64)
            .min(registry_next_id);
        let ids: Vec<u64> = (start..end).collect();
        let deposit_addresses = registry.get_addresses(&ids).await?;

        let deposit_addresses_ids: Vec<u64> = deposit_addresses.iter().map(|v| v.id).collect();
        if ids != deposit_addresses_ids {
            return Err(Error::MismatchingRawAddressIds);
        }

        for raw_deposit_address in deposit_addresses {
            let address_id = raw_deposit_address.id;
            match raw_deposit_address.try_into() {
                Ok(deposit_address) => {
                    store.add(deposit_address)?;
                    tracing::info!(address_id, "added new address from registry");
                }
                Err(Error::MissingAddressScripts) => {
                    tracing::debug!(address_id, "skipping empty address id");
                }
                Err(error) => tracing::warn!(
                    %error,
                    address_id,
                    "cannot parse deposit address"
                ),
            };
            store.set_last_next_address_id(address_id.saturating_add(1))?;
        }
    }

    Ok(())
}

fn update_wallet(context: &Context) -> Result<(), Error> {
    let Some(ref wallet) = context.settings().node_wallet else {
        return Ok(());
    };

    let script_pubkeys = context.storage().get_scripts()?;
    if script_pubkeys.is_empty() {
        return Ok(());
    }

    let timestamp = wallet.get_rescan_timestamp()?;

    import_descriptors(context.bitcoin_client(), &script_pubkeys, timestamp)
}

async fn fetch_and_create_deposits(
    context: &Context,
    deposit_monitor: &mut DepositMonitor,
    chain_tip: &BlockRef,
) -> Result<(), Error> {
    let emily_config = context.emily_config();

    let deposits = deposit_monitor.get_pending_deposits(chain_tip)?;

    tracing::debug!(count = deposits.len(), "fetched pending deposits");
    if deposits.is_empty() {
        return Ok(());
    }

    for deposit in deposits {
        if let Err(error) = deposit_api::create_deposit(emily_config, deposit.clone()).await {
            tracing::warn!(
                %error,
                txid = %deposit.bitcoin_txid,
                vout = %deposit.bitcoin_tx_output_index,
                "cannot create deposit in emily"
            );
        } else {
            tracing::info!(
                txid = %deposit.bitcoin_txid,
                vout = %deposit.bitcoin_tx_output_index,
                "created deposit in emily"
            );
            deposit_monitor.deposit_created(&deposit.bitcoin_txid, deposit.bitcoin_tx_output_index);
        }
    }

    Ok(())
}
