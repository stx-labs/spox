//! A module with structs that interact with the Stacks API.

use std::borrow::Cow;
use std::time::Duration;

use bitcoin::{PublicKey, XOnlyPublicKey};
use blockstack_lib::burnchains::Txid;
use blockstack_lib::chainstate::stacks::StacksTransaction;
use blockstack_lib::codec::StacksMessageCodec as _;
use clarity::types::chainstate::BlockHeaderHash;
use clarity::types::chainstate::ConsensusHash;
use clarity::types::chainstate::StacksAddress;
use clarity::vm::types::{BuffData, SequenceData};
use clarity::vm::{ClarityName, ContractName, Value};
use reqwest::header::CONTENT_LENGTH;
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Deserializer};
use url::Url;

use crate::config::Settings;
use crate::error::Error;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The response from a GET /v2/data_var/<contract-principal>/<contract-name>/<var-name> request.
#[derive(Debug, Deserialize)]
pub struct DataVarResponse {
    /// The value of the data variable.
    #[serde(deserialize_with = "clarity_value_deserializer")]
    pub data: Value,
}

/// The request body for a POST /v2/contracts/call-read/<contract-principal>/<contract-name>/<fn-name> request.
#[derive(Debug, serde::Serialize)]
pub struct CallReadRequest {
    /// The simulated address of the sender.
    pub sender: String,
    /// The arguments to the function in index-order.
    pub arguments: Vec<String>,
}

/// The response from a POST /v2/contracts/call-read/<contract-principal>/<contract-name>/<fn-name> request.
#[derive(Debug, Deserialize)]
pub struct CallReadResponse {
    /// The result of the function call.
    #[serde(deserialize_with = "clarity_value_deserializer")]
    pub result: Value,
}

/// JSON body returned by GET /v2/accounts/<principal>.
#[derive(Debug, Deserialize)]
struct AccountEntryResponse {
    /// Hex-encoded total balance in micro-STX, including locked funds.
    balance: String,
    /// Hex-encoded amount locked (stacked) in micro-STX.
    locked: String,
    /// Stacks block height at which the locked micro-STX unlock.
    unlock_height: u64,
    /// Next nonce for the account.
    nonce: u64,
}

/// Account info for a Stacks address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountInfo {
    /// The total balance of the account in micro-STX, including locked funds.
    pub balance: u128,
    /// The amount locked (stacked) in micro-STX.
    pub locked: u128,
    /// The height of the stacks block where the locked micro-STX unlock.
    pub unlock_height: u64,
    /// The next nonce for the account.
    pub nonce: u64,
}

/// The details of a rejected transaction
///
/// The fields match the JSON fields returned from a Stacks node and are
/// defined in:
/// https://github.com/stacks-network/stacks-core/blob/2.5.0.0.5/docs/rpc-endpoints.md
#[derive(Debug, Deserialize)]
pub struct TxRejection {
    /// The error message. It should always be the string "transaction
    /// rejection".
    pub error: String,
    /// The reason code for the rejection.
    pub reason: String,
    /// More details about the reason for the rejection.
    pub reason_data: Option<serde_json::Value>,
    /// The transaction ID of the rejected transaction.
    pub txid: Txid,
}

impl std::fmt::Display for TxRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "transaction rejected from the mempool: {}", self.reason)
    }
}

impl std::error::Error for TxRejection {}

/// The response from a POST /v2/transactions request.
///
/// The stacks node returns three types of responses, either:
/// 1. A 200 status hex encoded txid in the response body (on acceptance)
/// 2. A 400 status with a JSON object body (on rejection),
/// 3. A 400/500 status string message about some other error (such as
///    using an unsupported address mode).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum SubmitTxResponse {
    /// The transaction ID for the submitted transaction.
    Acceptance(Txid),
    /// The response when the transaction is rejected from the node.
    Rejection(TxRejection),
}

/// Subset of the response from `GET /v2/info`.
///
/// Despite the field name, `network_id` is the Stacks chain id used in
/// transactions.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct NodeInfo {
    /// Stacks chain id reported by the node.
    #[serde(rename = "network_id")]
    pub chain_id: u32,
    /// Block header hash of the tip of the canonical Stacks chain.
    pub stacks_tip: BlockHeaderHash,
    /// Consensus hash of the tip of the canonical Stacks chain.
    pub stacks_tip_consensus_hash: ConsensusHash,
}

/// Helper function for converting a hexadecimal string into an integer.
fn parse_hex_u128(hex: &str) -> Result<u128, Error> {
    let hex_str = hex.trim_start_matches("0x");
    u128::from_str_radix(hex_str, 16).map_err(Error::ParseHexInt)
}

impl TryFrom<AccountEntryResponse> for AccountInfo {
    type Error = Error;

    fn try_from(value: AccountEntryResponse) -> Result<Self, Self::Error> {
        Ok(AccountInfo {
            balance: parse_hex_u128(&value.balance)?,
            locked: parse_hex_u128(&value.locked)?,
            unlock_height: value.unlock_height,
            nonce: value.nonce,
        })
    }
}

/// A client for interacting with Stacks nodes and the Stacks API
#[derive(Debug, Clone)]
pub struct StacksClient {
    /// The base url for the Stacks node's RPC API.
    pub endpoint: Url,
    /// The client used to make the request.
    pub client: reqwest::Client,
}

impl StacksClient {
    /// Create a new instance of the Stacks client using the given
    /// StacksSettings.
    pub fn new(url: Url) -> Result<Self, Error> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()?;

        Ok(Self { endpoint: url, client })
    }

    /// Retrieve the latest value of a data variable from the specified contract.
    ///
    /// This is done by making a
    /// `GET /v2/data_var/<contract-principal>/<contract-name>/<var-name>`
    /// request. In the request we specify that the proof should not be included
    /// in the response.
    #[tracing::instrument(skip_all)]
    pub async fn get_data_var(
        &self,
        contract_principal: &StacksAddress,
        contract_name: &ContractName,
        var_name: &ClarityName,
    ) -> Result<Value, Error> {
        let path = format!("/v2/data_var/{contract_principal}/{contract_name}/{var_name}?proof=0");

        let url = self
            .endpoint
            .join(&path)
            .map_err(|err| Error::PathJoin(err, self.endpoint.clone(), Cow::Owned(path)))?;

        tracing::debug!(
            %contract_principal,
            %contract_name,
            %var_name,
            "fetching contract data variable"
        );

        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(Error::StacksNodeRequest)?;

        response
            .error_for_status()
            .map_err(Error::StacksNodeResponse)?
            .json::<DataVarResponse>()
            .await
            .map_err(Error::UnexpectedStacksResponse)
            .map(|x| x.data)
    }

    /// Calls a read-only public function on a given smart contract.
    #[tracing::instrument(skip_all)]
    pub async fn call_read(
        &self,
        contract_principal: &StacksAddress,
        contract_name: &ContractName,
        fn_name: &ClarityName,
        sender: &StacksAddress,
        arguments: &[Value],
    ) -> Result<Value, Error> {
        let path = format!(
            "/v2/contracts/call-read/{contract_principal}/{contract_name}/{fn_name}?tip=latest"
        );

        let url = self
            .endpoint
            .join(&path)
            .map_err(|err| Error::PathJoin(err, self.endpoint.clone(), Cow::Owned(path)))?;

        // Turns out that serializing clarity values to hex can panic. One
        // such case happens when the buff-data is too large, more than one
        // MBs worth. For our uses this should never happen.
        let arguments = arguments
            .iter()
            .map(|value| value.serialize_to_hex().map_err(Box::new))
            .collect::<Result<Vec<String>, _>>()
            .map_err(Error::ClarityValueSerialization)?;

        let body = CallReadRequest {
            sender: sender.to_string(),
            arguments,
        };

        tracing::debug!(
            %contract_principal,
            %contract_name,
            %fn_name,
            "Calling read-only function"
        );

        let response = self
            .client
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(Error::StacksNodeRequest)?;

        response
            .error_for_status()
            .map_err(Error::StacksNodeResponse)?
            .json::<CallReadResponse>()
            .await
            .map_err(Error::UnexpectedStacksResponse)
            .map(|x| x.result)
    }

    /// Get the latest account info for the given address.
    ///
    /// This is done by making a `GET /v2/accounts/<principal>` request. In
    /// the request we specify that the nonce and balance proofs should not
    /// be included in the response.
    #[tracing::instrument(skip_all)]
    pub async fn get_account(&self, address: &StacksAddress) -> Result<AccountInfo, Error> {
        let path = format!("/v2/accounts/{address}?proof=0");
        let url = self
            .endpoint
            .join(&path)
            .map_err(|err| Error::PathJoin(err, self.endpoint.clone(), Cow::Owned(path)))?;

        tracing::debug!(%address, "fetching the latest account information");

        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(Error::StacksNodeRequest)?;

        response
            .error_for_status()
            .map_err(Error::StacksNodeResponse)?
            .json::<AccountEntryResponse>()
            .await
            .map_err(Error::UnexpectedStacksResponse)
            .and_then(AccountInfo::try_from)
    }

    /// Get node info from `GET /v2/info`.
    ///
    /// Used to discover the chain id (`network_id`) for wallet and transaction
    /// construction whenever a stacks RPC endpoint is configured.
    #[tracing::instrument(skip_all)]
    pub async fn get_node_info(&self) -> Result<NodeInfo, Error> {
        let path = "/v2/info";
        let url = self
            .endpoint
            .join(path)
            .map_err(|err| Error::PathJoin(err, self.endpoint.clone(), Cow::Borrowed(path)))?;

        tracing::debug!("fetching stacks node info");

        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(Error::StacksNodeRequest)?;

        response
            .error_for_status()
            .map_err(Error::StacksNodeResponse)?
            .json::<NodeInfo>()
            .await
            .map_err(Error::UnexpectedStacksResponse)
    }

    /// Submit a transaction to a Stacks node.
    ///
    /// This is done by making a POST /v2/transactions request to a Stacks
    /// node. That endpoint supports two different content-types in the
    /// request body: JSON, and an octet-stream. This function always sends
    /// the raw transaction bytes as an octet-stream.
    #[tracing::instrument(skip_all)]
    pub async fn submit_tx(&self, tx: &StacksTransaction) -> Result<SubmitTxResponse, Error> {
        let path = "/v2/transactions";
        let url = self
            .endpoint
            .join(path)
            .map_err(|err| Error::PathJoin(err, self.endpoint.clone(), Cow::Borrowed(path)))?;

        tracing::debug!(txid = %tx.txid(), "submitting transaction to the stacks node");
        let body = tx.serialize_to_vec();

        let response: reqwest::Response = self
            .client
            .post(url)
            .timeout(REQUEST_TIMEOUT)
            .header(CONTENT_TYPE, "application/octet-stream")
            .header(CONTENT_LENGTH, body.len())
            .body(body)
            .send()
            .await
            .map_err(Error::StacksNodeRequest)?;

        response
            .json()
            .await
            .map_err(Error::UnexpectedStacksResponse)
    }

    /// Retrieve the current signers' aggregate key from the `sbtc-registry`
    /// contract.
    pub async fn get_current_signers_aggregate_key(
        &self,
        sbtc_deployer: &StacksAddress,
    ) -> Result<Option<XOnlyPublicKey>, Error> {
        let value = self
            .get_data_var(
                sbtc_deployer,
                &ContractName::from("sbtc-registry"),
                &ClarityName::from("current-aggregate-pubkey"),
            )
            .await?;

        extract_aggregate_key(value)
    }
}

impl TryFrom<&Settings> for StacksClient {
    type Error = Error;

    fn try_from(value: &Settings) -> Result<Self, Self::Error> {
        let stacks_config = value
            .stacks
            .as_ref()
            .ok_or_else(|| Error::MissingStacksConfig)?;

        StacksClient::new(stacks_config.rpc_endpoint.clone())
    }
}

/// A deserializer for Clarity's [`Value`] type that deserializes a hex-encoded
/// string which was serialized using Clarity's consensus serialization format.
fn clarity_value_deserializer<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    Value::try_deserialize_hex_untyped(&String::deserialize(deserializer)?)
        .map_err(serde::de::Error::custom)
}

/// Extract a aggregate key from a Clarity value.
///
/// In the sbtc-registry smart contract, the aggregate key is stored in the
/// `current-aggregate-pubkey` data var and is initialized to the 0x00
/// byte, allowing use to distinguish between the initial value and an
/// actual public key in that case. Ok(None) is returned if the value is
/// the initial value.
fn extract_aggregate_key(value: Value) -> Result<Option<XOnlyPublicKey>, Error> {
    match value {
        Value::Sequence(SequenceData::Buffer(BuffData { data })) => {
            // The initial value of the data var is all zeros
            if data.as_slice() == [0u8] {
                Ok(None)
            } else {
                PublicKey::from_slice(&data)
                    .map(|key| Some(XOnlyPublicKey::from(key)))
                    .map_err(Error::InvalidPublicKey)
            }
        }
        _ => Err(Error::InvalidStacksResponse(
            "expected a buffer but got something else",
        )),
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::SECP256K1;
    use bitcoin::{NetworkKind, PrivateKey};
    use clarity::types::Address;
    use clarity::vm::types::{BuffData, SequenceData};
    use test_case::test_case;

    use super::*;

    /// Helper method for generating a list of public keys.
    fn generate_pubkeys(count: u16) -> Vec<PublicKey> {
        (0..count)
            .map(|_| {
                PublicKey::from_private_key(SECP256K1, &PrivateKey::generate(NetworkKind::Test))
            })
            .collect()
    }

    #[test_case(false; "some")]
    #[test_case(true; "none")]
    #[tokio::test]
    async fn get_current_signers_aggregate_key_works(return_none: bool) {
        let aggregate_key = generate_pubkeys(1)[0];

        let data;
        let expected;
        if return_none {
            // 0x00 is the initial value of the signers' aggregate key in
            // the sbtc-registry contract, and
            // get_current_signers_aggregate_key should return None when we
            // receive it.
            data = vec![0];
            expected = None;
        } else {
            data = aggregate_key.inner.serialize().to_vec();
            expected = Some(aggregate_key.into());
        }
        let aggregate_key_clarity = Value::Sequence(SequenceData::Buffer(BuffData { data }));

        // The format of the response JSON is `{"data": "0x<serialized-value>"}` (excluding the proof).
        let raw_json_response = format!(
            r#"{{"data":"0x{}"}}"#,
            Value::serialize_to_hex(&aggregate_key_clarity).expect("failed to serialize value")
        );

        // Setup our mock server
        let mut stacks_node_server = mockito::Server::new_async().await;
        let mock = stacks_node_server
            .mock("GET", "/v2/data_var/ST1PQHQKV0RJXZFY1DGX8MNSNYVE3VGZJSRTPGZGM/sbtc-registry/current-aggregate-pubkey?proof=0")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&raw_json_response)
            .expect(1)
            .create();

        // Setup our Stacks client
        let client_url = url::Url::parse(stacks_node_server.url().as_str()).unwrap();
        let client = StacksClient::new(client_url).unwrap();

        let sbtc_deployer =
            StacksAddress::from_string("ST1PQHQKV0RJXZFY1DGX8MNSNYVE3VGZJSRTPGZGM").unwrap();

        // Make the request to the mock server
        let resp = client
            .get_current_signers_aggregate_key(&sbtc_deployer)
            .await
            .unwrap();

        // Assert that the response is what we expect
        assert_eq!(resp, expected);
        mock.assert();
    }

    #[test_case("0x1A3B5C7D9E", 112665066910; "uppercase-112665066910")]
    #[test_case("0x1a3b5c7d9e", 112665066910; "lowercase-112665066910")]
    #[test_case("1a3b5c7d9e", 112665066910; "no-prefix-lowercase-112665066910")]
    #[test_case("0xF0", 240; "uppercase-240")]
    #[test_case("f0", 240; "no-prefix-lowercase-240")]
    fn parsing_integers(hex: &str, expected: u128) {
        let actual = parse_hex_u128(hex).unwrap();
        assert_eq!(actual, expected);
    }

    #[test_case(""; "empty-string")]
    #[test_case("0x"; "almost-empty-string")]
    #[test_case("ZZZ"; "invalid hex")]
    fn parsing_integers_bad_input(hex: &str) {
        assert!(parse_hex_u128(hex).is_err());
    }

    #[tokio::test]
    async fn get_account_happy_path() {
        let address =
            StacksAddress::from_string("ST1PQHQKV0RJXZFY1DGX8MNSNYVE3VGZJSRTPGZGM").unwrap();

        let raw_json_response = r#"{
            "balance": "0x64",
            "locked": "0x0a",
            "unlock_height": 100,
            "nonce": 7
        }"#;

        let mut stacks_node_server = mockito::Server::new_async().await;
        let path = format!("/v2/accounts/{address}?proof=0");
        let mock = stacks_node_server
            .mock("GET", path.as_str())
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(raw_json_response)
            .expect(1)
            .create();

        let client_url = url::Url::parse(stacks_node_server.url().as_str()).unwrap();
        let client = StacksClient::new(client_url).unwrap();

        let account = client.get_account(&address).await.unwrap();
        assert_eq!(
            account,
            AccountInfo {
                balance: 100,
                locked: 10,
                unlock_height: 100,
                nonce: 7,
            }
        );
        mock.assert();
    }

    #[tokio::test]
    async fn get_node_info_returns_network_id() {
        // Minimal /v2/info body; serde ignores unknown fields.
        let raw_json_response = r#"{
            "peer_version": 4207599117,
            "pox_consensus": "dfe87cfd31c1a67fa8b989c83b79aa476e616758",
            "burn_block_height": 859080,
            "stable_pox_consensus": "c37cfa14d83c0e8dd87b8060adaf326dd60826d1",
            "stable_burn_block_height": 859073,
            "server_version": "stacks-node 0.0.1",
            "network_id": 2147483648,
            "parent_network_id": 3652501241,
            "stacks_tip_height": 123,
            "stacks_tip": "b5f9aa4423ffa7abb585fc00e2783c40225597ec112ee618db86ae23dbbbe88c",
            "stacks_tip_consensus_hash": "dfe87cfd31c1a67fa8b989c83b79aa476e616758",
            "genesis_chainstate_hash": "74237aa196aa178594804234417b84c7b4473cff69045cb7c5abf34e580ea427",
            "unanchored_tip": null,
            "unanchored_seq": null,
            "tenure_height": 100,
            "exit_at_block_height": null,
            "is_fully_synced": true
        }"#;

        let mut stacks_node_server = mockito::Server::new_async().await;
        let mock = stacks_node_server
            .mock("GET", "/v2/info")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(raw_json_response)
            .expect(1)
            .create();

        let client_url = url::Url::parse(stacks_node_server.url().as_str()).unwrap();
        let client = StacksClient::new(client_url).unwrap();

        let info = client.get_node_info().await.unwrap();
        assert_eq!(info.chain_id, blockstack_lib::core::CHAIN_ID_TESTNET);
        assert_eq!(
            info.stacks_tip,
            BlockHeaderHash::from_hex(
                "b5f9aa4423ffa7abb585fc00e2783c40225597ec112ee618db86ae23dbbbe88c"
            )
            .unwrap()
        );
        assert_eq!(
            info.stacks_tip_consensus_hash,
            ConsensusHash::from_hex("dfe87cfd31c1a67fa8b989c83b79aa476e616758").unwrap()
        );
        mock.assert();
    }
}
