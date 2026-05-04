//! Contains client wrappers for bitcoin core

use std::sync::Arc;
use std::time::Duration;

use bitcoin::ScriptBuf;
use bitcoincore_rpc::RpcApi;
use bitcoincore_rpc::jsonrpc;
use bitcoincore_rpc_json::{GetChainTipsResultStatus, ScanTxOutRequest, Utxo as RpcUtxo};
use percent_encoding::percent_decode_str;
use url::Url;

use crate::bitcoin::{BlockRef, Utxo};
use crate::error::Error;

impl From<RpcUtxo> for Utxo {
    fn from(value: RpcUtxo) -> Self {
        Utxo {
            txid: value.txid,
            vout: value.vout,
            script_pub_key: value.script_pub_key,
            amount: value.amount,
            block_height: value.height,
        }
    }
}

/// A client for interacting with bitcoin-core
#[derive(Clone)]
pub struct BitcoinCoreClient {
    /// The underlying bitcoin-core client
    inner: Arc<bitcoincore_rpc::Client>,
}

impl BitcoinCoreClient {
    /// Build a bitcoin-core RPC client from a URL with embedded credentials
    /// and a request timeout.
    ///
    /// `scantxoutset` is inherently long-running (1–10 min on typical hardware
    /// against a full UTXO set). The `jsonrpc` crate's default timeout of 15 s
    /// is too short and would abort the request mid-scan; bitcoind would keep
    /// scanning in the background and the next request would reject with
    /// `-8 "Scan already in progress"`. Callers should pass a timeout that
    /// comfortably exceeds the worst-case scan duration on their hardware.
    pub fn connect(url: &Url, timeout: Duration) -> Result<Self, Error> {
        // The `url` crate returns userinfo in its percent-encoded form. Decode
        // it before forwarding to bitcoind so that passwords containing
        // reserved characters (`=`, `+`, `@`, `:`, ...) authenticate correctly.
        let username = percent_decode_str(url.username())
            .decode_utf8()
            .map_err(|_| Error::InvalidUrl(url::ParseError::IdnaError))?
            .into_owned();
        let password = percent_decode_str(url.password().unwrap_or_default())
            .decode_utf8()
            .map_err(|_| Error::InvalidUrl(url::ParseError::IdnaError))?
            .into_owned();
        let host = url
            .host_str()
            .ok_or(Error::InvalidUrl(url::ParseError::EmptyHost))?;
        let port = url.port().ok_or(Error::PortRequired)?;
        let endpoint = format!("{}://{host}:{port}", url.scheme());

        let transport = jsonrpc::simple_http::SimpleHttpTransport::builder()
            .url(&endpoint)
            .map_err(|err| {
                Error::BitcoinCoreRpcClient(
                    bitcoincore_rpc::Error::JsonRpc(jsonrpc::Error::Transport(Box::new(err))),
                    endpoint.clone(),
                )
            })?
            .auth(username, Some(password))
            .timeout(timeout)
            .build();
        let client =
            bitcoincore_rpc::Client::from_jsonrpc(jsonrpc::Client::with_transport(transport));

        Ok(Self { inner: Arc::new(client) })
    }

    /// Get the canonical chain tip
    pub fn get_chain_tip(&self) -> Result<BlockRef, Error> {
        let result = self
            .inner
            .get_chain_tips()
            .map_err(Error::BitcoinCoreRpc)?
            .into_iter()
            .find(|tip| tip.status == GetChainTipsResultStatus::Active)
            .ok_or(Error::NoChainTip)?;

        Ok(BlockRef {
            block_hash: result.hash,
            block_height: result.height,
        })
    }

    /// Returns `true` if a `scantxoutset` is currently running on the
    /// bitcoind node (started by this client or by any other RPC user).
    ///
    /// Bitcoin Core serializes scans globally: only one can run at a time,
    /// and concurrent attempts fail with `-8 "Scan already in progress"`.
    /// Callers should pre-check this and skip work when busy.
    pub fn scantxoutset_in_progress(&self) -> Result<bool, Error> {
        let status: serde_json::Value = self
            .inner
            .call("scantxoutset", &["status".into()])
            .map_err(Error::BitcoinCoreRpc)?;
        Ok(!status.is_null())
    }

    /// Get UTXOs for addresses.
    ///
    /// Returns [`Error::ScanTxOutInProgress`] when a scan is already running
    /// on the node, so the caller can retry on the next poll instead of
    /// racing into a `-8` error.
    pub fn get_utxos<'a, I>(&self, addresses: I) -> Result<Vec<Utxo>, Error>
    where
        I: IntoIterator<Item = &'a ScriptBuf>,
    {
        if self.scantxoutset_in_progress()? {
            return Err(Error::ScanTxOutInProgress);
        }

        let descriptors = addresses
            .into_iter()
            .map(|addr| ScanTxOutRequest::Single(format!("raw({})", addr.to_hex_string())))
            .collect::<Vec<_>>();

        let result =
            self.inner
                .scan_tx_out_set_blocking(&descriptors)
                .map_err(|err| match &err {
                    bitcoincore_rpc::Error::JsonRpc(bitcoincore_rpc::jsonrpc::Error::Rpc(rpc))
                        if rpc.code == -8 =>
                    {
                        Error::ScanTxOutInProgress
                    }
                    _ => Error::BitcoinCoreRpc(err),
                })?;

        if result.success != Some(true) {
            return Err(Error::ScanTxOutFailure);
        }

        Ok(result.unspents.into_iter().map(Into::into).collect())
    }

    /// Get the canonical block hash for a given block height
    pub fn get_block_hash(&self, block_height: u64) -> Result<bitcoin::BlockHash, Error> {
        self.inner
            .get_block_hash(block_height)
            .map_err(Error::BitcoinCoreRpc)
    }

    /// Get the transaction hex
    pub fn get_raw_transaction_hex(
        &self,
        txid: &bitcoin::Txid,
        block_hash: &bitcoin::BlockHash,
    ) -> Result<String, Error> {
        self.inner
            .get_raw_transaction_hex(txid, Some(block_hash))
            .map_err(Error::BitcoinCoreRpc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encoded_password_is_decoded() {
        let url: Url = "http://user:pa%3Dss@host:8332/".parse().unwrap();
        assert_eq!(url.password(), Some("pa%3Dss"));

        let decoded = percent_decode_str(url.password().unwrap())
            .decode_utf8()
            .unwrap()
            .into_owned();
        assert_eq!(decoded, "pa=ss");
    }

    #[test]
    fn plain_password_passes_through() {
        let url: Url = "http://user:plain@host:8332/".parse().unwrap();
        let decoded = percent_decode_str(url.password().unwrap())
            .decode_utf8()
            .unwrap()
            .into_owned();
        assert_eq!(decoded, "plain");
    }
}
