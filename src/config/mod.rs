//! sPoX Configuration
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use bitcoin::{ScriptBuf, XOnlyPublicKey};
use bitcoincore_rpc_json::Timestamp;
use clarity::types::chainstate::StacksAddress;
use clarity::vm::types::{PrincipalData, QualifiedContractIdentifier};
use config::{Config, Environment, File};
use secp256k1::SecretKey;
use serde::Deserialize;
use url::Url;

use crate::config::error::SpoxConfigError;
use crate::config::serialization::{
    contract_deserializer, contract_deserializer_option, duration_seconds_deserializer,
    principal_deserializer, script_deserializer, stacks_address_deserializer, url_deserializer,
    xonly_deserializer,
};

pub mod error;
mod serialization;

/// Config environment variables prefix
pub const CONFIG_PREFIX: &str = "SPOX";

/// A monitored deposit config
#[derive(Deserialize, Clone, Debug)]
pub struct MonitoredDepositConfig {
    /// The signers xonly aggregate key
    #[serde(deserialize_with = "xonly_deserializer")]
    pub signers_xonly: XOnlyPublicKey,
    /// The deposit recipient
    #[serde(deserialize_with = "principal_deserializer")]
    pub recipient: PrincipalData,
    /// The deposit max fee
    pub max_fee: u64,
    /// The reclaim lock time
    pub lock_time: u32,
    /// The free portion of the reclaim script (after the `<lockTime> OP_CSV` prefix)
    #[serde(deserialize_with = "script_deserializer")]
    pub reclaim_script: ScriptBuf,
}

/// Top-level configuration
#[derive(Deserialize, Clone, Debug)]
pub struct Settings {
    /// Bitcoin RPC endpoint
    #[serde(deserialize_with = "url_deserializer")]
    pub bitcoin_rpc_endpoint: Url,
    /// Bitcoin RPC timeout
    #[serde(deserialize_with = "duration_seconds_deserializer")]
    pub bitcoin_rpc_timeout: std::time::Duration,
    /// Emily API endpoint
    #[serde(deserialize_with = "url_deserializer")]
    pub emily_endpoint: Url,
    /// How often looking for new deposit transactions
    #[serde(deserialize_with = "duration_seconds_deserializer")]
    pub polling_interval: std::time::Duration,
    /// Monitored deposits
    #[serde(default)]
    pub deposit: HashMap<String, MonitoredDepositConfig>,
    /// Registry smart contract address
    #[serde(default, deserialize_with = "contract_deserializer_option")]
    pub registry_contract: Option<QualifiedContractIdentifier>,
    /// Reward-claim / settlement config.
    ///
    /// Presence of this stanza enables the reward-claim process. Requires
    /// [`Settings::stacks`] as well.
    pub reward_claims: Option<RewardClaimsConfig>,
    /// Stacks config, used for CLI commands, registry reads, and reward claims.
    pub stacks: Option<StacksConfig>,
    /// Bitcoin core wallet config
    pub node_wallet: Option<BitcoinCoreWalletConfig>,
}

/// Stacks related config.
#[derive(Deserialize, Clone, Debug)]
pub struct StacksConfig {
    /// Stacks rpc endpoint
    #[serde(deserialize_with = "url_deserializer")]
    pub rpc_endpoint: Url,
    /// The address of the deployer of the sBTC smart contracts.
    #[serde(deserialize_with = "stacks_address_deserializer")]
    pub sbtc_deployer: StacksAddress,
}

/// Config for signing and submitting reward-claim transactions.
///
/// When this stanza is present, `[stacks]` must also be configured.
#[derive(Deserialize, Clone, Debug)]
pub struct RewardClaimsConfig {
    /// Fully-qualified reward-claims smart contract identifier.
    #[serde(deserialize_with = "contract_deserializer")]
    pub claims_contract: QualifiedContractIdentifier,
    /// The private key used to sign contract call transactions to the
    /// rewards-claim registry.
    pub private_key: SecretKey,
}

/// Bitcoin core wallet config.
#[derive(Deserialize, Clone, Debug)]
pub struct BitcoinCoreWalletConfig {
    /// Bitcoin core wallet name, managed by spox
    pub name: String,
    /// Timestamp used for rescans when importing new descriptors.
    ///
    /// Non-negative values are UNIX timestamps (in seconds); `0` scans from
    /// genesis.
    /// Negative values can be used to specify offsets from current UNIX
    /// timestamp (e.g., `-3600` to use `now - 1 hour`).
    pub rescan_timestamp: i64,
}

impl Settings {
    /// Initializing the global config first with default values and then with
    /// provided/overwritten environment variables. The explicit separator with
    /// double underscores is needed to correctly parse the nested config structure.
    ///
    /// The environment variables are prefixed with `SPOX_` and the nested
    /// fields are separated with double underscores.
    pub fn new(config_path: Option<impl AsRef<Path>>) -> Result<Self, SpoxConfigError> {
        let env = Environment::with_prefix(CONFIG_PREFIX)
            .prefix_separator("_")
            .separator("__");

        let mut cfg_builder = Config::builder();

        cfg_builder = cfg_builder.set_default("polling_interval", 30)?;
        cfg_builder = cfg_builder.set_default("bitcoin_rpc_timeout", 60 * 5)?;

        if let Some(path) = config_path {
            cfg_builder = cfg_builder.add_source(File::from(path.as_ref()));
        }
        cfg_builder = cfg_builder.add_source(env);

        let cfg = cfg_builder.build()?;

        let settings: Settings = cfg.try_deserialize()?;

        settings.validate()?;

        Ok(settings)
    }

    /// Perform validation on the configuration.
    fn validate(&self) -> Result<(), SpoxConfigError> {
        if self.polling_interval.is_zero() {
            return Err(SpoxConfigError::ZeroDurationForbidden("polling_interval"));
        }
        if self.bitcoin_rpc_timeout.is_zero() {
            return Err(SpoxConfigError::ZeroDurationForbidden(
                "bitcoin_rpc_timeout",
            ));
        }

        if self.registry_contract.is_some() && self.stacks.is_none() {
            return Err(SpoxConfigError::MissingStacksConfig);
        }

        if self.reward_claims.is_some() && self.stacks.is_none() {
            return Err(SpoxConfigError::MissingStacksConfig);
        }

        if let Some(ref wallet) = self.node_wallet
            && wallet.name.trim().is_empty()
        {
            return Err(SpoxConfigError::EmptyBitcoinWalletName);
        }

        Ok(())
    }
}

impl BitcoinCoreWalletConfig {
    /// Get the rescan timestamp to be used when importing descriptors:
    /// non-negative values of `rescan_timestamp` are treated as UNIX timestamps,
    /// negative values are treated as offsets from the current UNIX timestamp.
    pub fn get_rescan_timestamp(&self) -> Result<Timestamp, crate::error::Error> {
        if self.rescan_timestamp >= 0 {
            return Ok(Timestamp::Time(self.rescan_timestamp as u64));
        }

        let Ok(current_timestamp) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            return Err(crate::error::Error::UnexpectedLocalTimestamp);
        };

        let current_timestamp = current_timestamp.as_secs();
        Ok(Timestamp::Time(
            current_timestamp.saturating_add_signed(self.rescan_timestamp),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use test_case::test_case;

    use super::*;
    use crate::testing::{clear_env, set_var};

    /// Helper function to quickly create a URL from a string in tests.
    fn parse_url(s: &str) -> url::Url {
        s.parse().unwrap()
    }

    /// This test checks that the default configuration values are loaded
    /// correctly from the default.toml file. The Stacks settings are excluded
    /// as they are covered by the [`default_config_toml_loads_with_environment`]
    /// test.
    // !! NOTE: This test needs to be updated if the default values in the
    // !! default.toml file are changed.
    #[test]
    fn default_config_toml_loads() {
        clear_env();

        let settings = Settings::new_from_default_config()
            .expect("Failed create settings from default config");

        assert_eq!(settings.emily_endpoint, parse_url("http://127.0.0.1:3031"));
        assert_eq!(
            settings.bitcoin_rpc_endpoint,
            parse_url("http://devnet:devnet@127.0.0.1:18443")
        );
        assert_eq!(settings.polling_interval, Duration::from_secs(30));
        assert_eq!(settings.bitcoin_rpc_timeout, Duration::from_mins(5));
        assert!(settings.registry_contract.is_none());
        assert!(settings.reward_claims.is_none());
        settings.stacks.as_ref().unwrap();
    }

    #[test]
    fn default_config_toml_loads_with_environment() {
        clear_env();

        set_var("SPOX_POLLING_INTERVAL", "31");

        let settings = Settings::new_from_default_config().unwrap();

        assert_eq!(settings.polling_interval, Duration::from_secs(31));
    }

    #[test_case("bitcoin_rpc_endpoint"; "bitcoin_rpc_endpoint")]
    #[test_case("emily_endpoint"; "emily_endpoint")]
    fn parsing_url_error(field: &str) {
        clear_env();

        set_var(format!("SPOX_{}", field.to_uppercase()), "not a url");

        assert!(matches!(
            Settings::new_from_default_config(),
            Err(SpoxConfigError::ConfigError(_))
        ));
    }

    #[test_case("polling_interval"; "polling interval")]
    #[test_case("bitcoin_rpc_timeout"; "bitcoin rpc timeout")]
    fn zero_values_for_nonzero_fields_fail_in_config(field: &str) {
        clear_env();

        set_var(format!("SPOX_{}", field.to_uppercase()), "0");

        Settings::new_from_default_config().expect_err("value for must be non zero");
    }

    #[test]
    fn parsing_registry_contract() {
        clear_env();

        let registry = "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.registry";
        set_var("SPOX_REGISTRY_CONTRACT", registry);

        let settings = Settings::new_from_default_config().unwrap();

        assert_eq!(settings.registry_contract.unwrap().to_string(), registry);
    }

    #[test_case("not an address"; "not an address")]
    #[test_case("ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039"; "not a contract")]
    fn parsing_registry_contract_fails(registry: &str) {
        clear_env();

        set_var("SPOX_REGISTRY_CONTRACT", registry);

        assert!(matches!(
            Settings::new_from_default_config(),
            Err(SpoxConfigError::ConfigError(_))
        ));
    }

    #[test_case(4933574768; "year 2126")]
    #[test_case(100; "100")]
    #[test_case(0; "0")]
    fn non_negative_timestamp(timestamp: u64) {
        clear_env();

        set_var("SPOX_NODE_WALLET__RESCAN_TIMESTAMP", timestamp.to_string());
        let config = Settings::new_from_default_config().unwrap();

        assert_eq!(
            config.node_wallet.unwrap().get_rescan_timestamp().unwrap(),
            Timestamp::Time(timestamp)
        );
    }

    #[test]
    fn negative_timestamp() {
        clear_env();

        set_var("SPOX_NODE_WALLET__RESCAN_TIMESTAMP", "-100");
        let config = Settings::new_from_default_config().unwrap();

        let Timestamp::Time(config_timestamp) =
            config.node_wallet.unwrap().get_rescan_timestamp().unwrap()
        else {
            panic!("wrong timestamp")
        };

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(100);
        assert!(timestamp.abs_diff(config_timestamp) <= 2);
    }

    #[test]
    fn empty_wallet_name() {
        clear_env();

        set_var("SPOX_NODE_WALLET__NAME", "");

        assert!(matches!(
            Settings::new_from_default_config(),
            Err(SpoxConfigError::EmptyBitcoinWalletName)
        ));
    }

    #[test]
    fn reward_claims_requires_stacks_config() {
        clear_env();

        // Provide a complete [reward_claims] via env but clear stacks by using a
        // minimal config path is hard; instead set reward_claims fields and
        // rely on default.toml still having [stacks]. Use validate directly.
        let mut settings = Settings::new_from_default_config().unwrap();
        settings.stacks = None;
        settings.reward_claims = Some(RewardClaimsConfig {
            claims_contract: QualifiedContractIdentifier::parse(
                "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.reward-claims",
            )
            .unwrap(),
            private_key: SecretKey::from_slice(&[0xa0; 32]).unwrap(),
        });

        assert!(matches!(
            settings.validate(),
            Err(SpoxConfigError::MissingStacksConfig)
        ));
    }

    #[test]
    fn reward_claims_stanza_loads() {
        clear_env();

        set_var(
            "SPOX_REWARD_CLAIMS__CLAIMS_CONTRACT",
            "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.reward-claims",
        );

        set_var(
            "SPOX_REWARD_CLAIMS__PRIVATE_KEY",
            "0000000000000000000000000000000000000000000000000000000000000001",
        );

        let settings = Settings::new_from_default_config().unwrap();
        let claims = settings.reward_claims.unwrap();
        assert_eq!(
            claims.claims_contract.to_string(),
            "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.reward-claims"
        );
        assert!(settings.stacks.is_some());
    }
}
