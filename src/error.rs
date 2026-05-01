//! Top-level error type

use std::borrow::Cow;

use bitcoin::ScriptBuf;

/// Top-level application error
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Error from the Bitcoin RPC client.
    #[error("bitcoin RPC error: {0}")]
    BitcoinCoreRpc(#[from] bitcoincore_rpc::Error),

    /// Error when creating an RPC client to bitcoin-core
    #[error("could not create RPC client to {1}: {0}")]
    BitcoinCoreRpcClient(#[source] bitcoincore_rpc::Error, String),

    /// Could not serialize the clarity value to bytes.
    ///
    /// For some reason, InterpreterError does not implement
    /// std::fmt::Display or std::error::Error, hence the debug log.
    #[error("clarity serialization error: {0:?}")]
    ClarityValueSerialization(Box<clarity::vm::errors::InterpreterError>),

    /// Could not create a clarity list. This shouldn't happen.
    #[error("clarity bad list: {0:?}")]
    ClarityBadList(Box<clarity::vm::errors::Error>),

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

    /// Poisoned mutex
    #[error("poisoned mutex")]
    PoisonedMutex,

    /// No chain tip found.
    #[error("no bitcoin chain tip")]
    NoChainTip,

    /// No signers aggregate key configured in the registry
    #[error("no signers aggregate key configured in the registry")]
    NoSignersAggregateKey,

    /// Missing monitored deposit address for scriptPubKey
    #[error("missing monitored deposit address for scriptPubKey {0}")]
    MissingMonitoredDeposit(ScriptBuf),

    /// Missing stacks configuration
    #[error("missing stacks configuration")]
    MissingStacksConfig,

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

    /// A call to `scantxoutset` failed
    #[error("a call to `scantxoutset` failed")]
    ScanTxOutFailure,

    /// A `scantxoutset` is already running on the bitcoind node. The current
    /// poll iteration is skipped and the scan will be retried on the next one.
    #[error("a `scantxoutset` is already in progress on the bitcoin node; skipping this poll")]
    ScanTxOutInProgress,

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

    /// Registry returned ids that do not match the requested ids
    #[error("registry returned ids that do not match the requested ids")]
    MismatchingRawAddressIds,
}
