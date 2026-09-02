use std::str::FromStr as _;

use bitcoin::{ScriptBuf, XOnlyPublicKey, secp256k1};
use clarity::vm::types::PrincipalData;
use clarity::{types::chainstate::StacksAddress, vm::types::QualifiedContractIdentifier};
use serde::{Deserialize, Deserializer};

/// A deserializer for the url::Url type. Does not support deserializing a list,
/// only a single URL.
pub fn url_deserializer<'de, D>(deserializer: D) -> Result<url::Url, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer)?
        .parse()
        .map_err(serde::de::Error::custom)
}

/// A deserializer for the std::time::Duration type.
/// Serde includes a default deserializer, but it expects a struct.
pub fn duration_seconds_deserializer<'de, D>(
    deserializer: D,
) -> Result<std::time::Duration, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(std::time::Duration::from_secs(
        u64::deserialize(deserializer).map_err(serde::de::Error::custom)?,
    ))
}

/// Parse the string into a StacksAddress.
///
/// The [`StacksAddress`] struct does not implement any string parsing or
/// c32 decoding. However, the [`PrincipalData::parse_standard_principal`]
/// function does the expected c32 decoding and the validation, so we go
/// through that.
pub fn stacks_address_deserializer<'de, D>(des: D) -> Result<StacksAddress, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let literal = <String>::deserialize(des)?;

    PrincipalData::parse_standard_principal(&literal)
        .map(StacksAddress::from)
        .map_err(serde::de::Error::custom)
}

/// Parse the string into a Stacks PrincipalData.
pub fn principal_deserializer<'de, D>(des: D) -> Result<PrincipalData, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let literal = <String>::deserialize(des)?;
    PrincipalData::parse(&literal).map_err(serde::de::Error::custom)
}

/// Parse an optional string into a Stacks QualifiedContractIdentifier.
pub fn contract_deserializer_option<'de, D>(
    des: D,
) -> Result<Option<QualifiedContractIdentifier>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let literal = <Option<String>>::deserialize(des)?.filter(|principal| !principal.is_empty());
    let Some(principal) = literal else {
        return Ok(None);
    };

    QualifiedContractIdentifier::parse(&principal)
        .map(Some)
        .map_err(serde::de::Error::custom)
}

/// Parse the string into a Stacks QualifiedContractIdentifier.
pub fn contract_deserializer<'de, D>(des: D) -> Result<QualifiedContractIdentifier, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let literal = <String>::deserialize(des)?;
    QualifiedContractIdentifier::parse(&literal).map_err(serde::de::Error::custom)
}

/// Parse the string into a XOnlyPublicKey
pub fn xonly_deserializer<'de, D>(des: D) -> Result<XOnlyPublicKey, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let literal = <String>::deserialize(des)?;
    secp256k1::XOnlyPublicKey::from_str(&literal).map_err(serde::de::Error::custom)
}

/// Parse the string into a ScriptBuf
pub fn script_deserializer<'de, D>(des: D) -> Result<ScriptBuf, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let literal = <String>::deserialize(des)?;
    ScriptBuf::from_hex(&literal).map_err(serde::de::Error::custom)
}
