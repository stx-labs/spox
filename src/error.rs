//! Top-level error type

use std::borrow::Cow;

use bitcoin::ScriptBuf;
use bitcoincore_rpc::jsonrpc;

/// Top-level application error
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Error from the Bitcoin RPC client.
    #[error("bitcoin RPC error: {0}")]
    BitcoinCoreRpc(#[from] bitcoincore_rpc::Error),

    /// Error when creating an RPC client to bitcoin-core
    #[error("could not create RPC client to {1}: {0}")]
    BitcoinCoreRpcClient(#[source] jsonrpc::http::simple_http::Error, String),

    /// Could not deserialize the clarity value from a hex-encoded string.
    #[error("clarity deserialization error: {0:?}")]
    ClarityValueDeserialization(Box<clarity::vm::types::serialization::SerializationError>),

    /// Could not serialize the clarity value to bytes.
    #[error("clarity serialization error: {0:?}")]
    ClarityValueSerialization(Box<clarity::vm::types::serialization::SerializationError>),

    /// Could not create a clarity list. This shouldn't happen.
    #[error("clarity bad list: {0:?}")]
    ClarityBadList(Box<clarity::vm::errors::ClarityTypeError>),

    /// Could not construct a clarity value.
    #[error("clarity construction error: {0:?}")]
    ClarityTuple(Box<clarity::vm::errors::ClarityTypeError>),

    /// Missing an expected tuple entry. This shouldn't happen.
    #[error("missing an expected tuple entry: {0}")]
    ClarityMissingTupleEntry(&'static str),

    /// The raw deposit is missing the address scripts
    #[error("the raw deposit is missing the address scripts")]
    MissingAddressScripts,

    /// The pending deposit is expired
    #[error("the pending deposit is expired")]
    DepositExpired,

    /// This occurs when converting a byte slice to a secp256k1::PublicKey.
    #[error("invalid public key: {0}")]
    InvalidPublicKey(#[source] bitcoin::key::FromSliceError),

    /// The response from the Stacks node was invalid or malformed.
    #[error("invalid stacks response: {0}")]
    InvalidStacksResponse(&'static str),

    /// Error when parsing a URL
    #[error("could not parse the provided URL: {0}")]
    InvalidUrl(#[source] url::ParseError),

    /// The Stacks RPC auth token cannot be used as an HTTP `Authorization`
    /// header value.
    #[error("stacks rpc auth token is not a valid HTTP header value: {0}")]
    InvalidStacksAuthToken(#[source] reqwest::header::InvalidHeaderValue),

    /// Poisoned mutex
    #[error("poisoned mutex")]
    PoisonedMutex,

    /// No chain tip found.
    #[error("no bitcoin chain tip")]
    NoChainTip,

    /// No signers aggregate key configured in the sBTC registry
    #[error("no signers aggregate key configured in the sBTC registry")]
    NoSignersAggregateKey,

    /// Missing monitored deposit address for scriptPubKey
    #[error("missing monitored deposit address for scriptPubKey {0}")]
    MissingMonitoredDeposit(ScriptBuf),

    /// Missing stacks configuration
    #[error("missing stacks configuration")]
    MissingStacksConfig,

    /// Missing reward-claims configuration
    #[error("missing reward-claims configuration")]
    MissingRewardClaimsConfig,

    /// No registry contract configured
    #[error("no registry contract configured")]
    NoRegistryConfigured,

    /// Could not parse the path part of a URL
    #[error("failed to construct a valid URL from {1} and {2}: {0}")]
    PathJoin(#[source] url::ParseError, url::Url, Cow<'static, str>),

    /// Error when the port is not provided
    #[error("a port must be specified")]
    PortRequired,

    /// Reqwest error
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),

    /// sBTC error
    #[error(transparent)]
    Sbtc(#[from] sbtc::error::Error),

    /// Scan already in progress
    #[error("scan already in progress")]
    ScanAlreadyInProgress,

    /// A call to `scantxoutset` failed
    #[error("a call to `scantxoutset` failed")]
    ScanTxOutFailure,

    /// Could not make a successful request to the Stacks node.
    #[error("failed to make a request to the stacks Node: {0}")]
    StacksNodeRequest(#[source] reqwest::Error),

    /// Could not make a successful request to the stacks API.
    #[error("received a non success status code response from a stacks node: {0}")]
    StacksNodeResponse(#[source] reqwest::Error),

    /// Tried to fetch too many registered addresses in a single call
    #[error("too many ids to fetch: {0}, max size is {1}")]
    TooManyAddressIDs(usize, u32),

    /// Reqwest error
    #[error("response from stacks node did not conform to the expected schema: {0}")]
    UnexpectedStacksResponse(#[source] reqwest::Error),

    /// This variant is for when the clarity principal returned from our
    /// read-only call for the signer manager is not a qualitfied contract
    /// identifier. This should never happen, seeing it means we have a bug
    /// in the smart contract.
    #[error("the clarity principal was not a smart contract principal")]
    UnexpectedPrincipal(clarity::vm::types::PrincipalData),

    /// Unexpected local timestamp
    #[error("unexpected local timestamp")]
    UnexpectedLocalTimestamp,

    /// Registry returned ids that do not match the requested ids
    #[error("registry returned ids that do not match the requested ids")]
    MismatchingRawAddressIds,

    /// Failed to parse a hex-encoded integer from a Stacks node response.
    #[error("could not parse hex integer: {0}")]
    ParseHexInt(#[source] std::num::ParseIntError),

    /// Failed to parse a JSON response from the Stacks node.
    #[error("failed to transform a PrincipalData into or from JSON: {0}")]
    PrincipalDataSerdeJson(#[source] serde_json::Error),

    /// A `/v3/contracts/fast-call-read` request returned `okay: false`.
    ///
    /// The node still answers HTTP 200; the Clarity VM error is in `cause`.
    #[error("read-only stacks call failed: {0:?}")]
    ReadOnlyCallFailed(Option<String>),
}
