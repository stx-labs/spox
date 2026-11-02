//! Contains wallet operations

use std::collections::HashSet;

use bitcoin::ScriptBuf;
use bitcoincore_rpc::Error as BtcRpcError;
use bitcoincore_rpc::jsonrpc::error::Error as JsonRpcError;
use bitcoincore_rpc::jsonrpc::error::RpcError;
use bitcoincore_rpc_json::ImportDescriptors;
use bitcoincore_rpc_json::Timestamp;

use crate::bitcoin::node::BitcoinCoreClient;
use crate::error::Error;

/// Strip the checksum suffix from a descriptor
fn strip_checksum(descriptor: &str) -> &str {
    descriptor
        .rsplit_once('#')
        .map(|(d, _)| d)
        .unwrap_or(descriptor)
}

/// Load the Bitcoin wallet, creating it if it doesn't exist.
pub fn load_or_create_wallet(bitcoin: &BitcoinCoreClient, wallet: &str) -> Result<(), Error> {
    match bitcoin.load_wallet(wallet) {
        Ok(_) => Ok(()),
        // The wallet has already been loaded
        Err(Error::BitcoinCoreRpc(BtcRpcError::JsonRpc(JsonRpcError::Rpc(RpcError {
            code: -35,
            ..
        })))) => Ok(()),
        // The wallet doesn't exist
        Err(Error::BitcoinCoreRpc(BtcRpcError::JsonRpc(JsonRpcError::Rpc(RpcError {
            code: -18,
            ..
        })))) => bitcoin.create_wallet(wallet).map(|result| {
            tracing::info!(wallet = result.name, "created bitcoin wallet");
        }),
        Err(e) => Err(e),
    }
}

/// Import the missing descriptors in the Bitcoin wallet
pub fn import_descriptors(
    bitcoin: &BitcoinCoreClient,
    script_pubkeys: &[ScriptBuf],
    rescan_timestamp: Timestamp,
) -> Result<(), Error> {
    let descriptors: HashSet<String> = bitcoin
        .list_descriptors()?
        .descriptors
        .into_iter()
        .map(|entry| strip_checksum(&entry.desc).to_owned())
        .collect();

    let mut new_descriptors = Vec::new();

    for script in script_pubkeys {
        let desc = format!("raw({})", script.to_hex_string());
        if descriptors.contains(&desc) {
            continue;
        }

        let descriptor_info = match bitcoin.get_descriptor_info(&desc) {
            Ok(descriptor_info) => descriptor_info,
            Err(error) => {
                tracing::warn!(
                    %error,
                    descriptor=desc,
                    "error getting descriptor info; skipping importing it, will retry later"
                );
                continue;
            }
        };

        new_descriptors.push(ImportDescriptors {
            descriptor: descriptor_info.descriptor,
            timestamp: rescan_timestamp,
            internal: Some(false),
            active: None,
            range: None,
            next_index: None,
            label: None,
        });
    }

    if new_descriptors.is_empty() {
        return Ok(());
    }

    // TODO: batch the descriptors in case there are too many for a single call
    let results = bitcoin.import_descriptors(&new_descriptors)?;

    let mut imported = 0;
    let mut errors = 0;
    for result in results {
        if result.success {
            imported += 1;
        } else {
            errors += 1;
            tracing::warn!(error=?result.error, "error importing descriptor");
        }
    }
    tracing::info!(imported, errors, "imported descriptors");

    Ok(())
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;

    #[test_case("raw(0123)#abcd", "raw(0123)"; "with checksum")]
    #[test_case("raw(0123)", "raw(0123)"; "no checksum")]
    fn strip_checksum_works(input: &str, expected: &str) {
        assert_eq!(strip_checksum(input), expected);
    }
}
