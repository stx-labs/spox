//! sPoX Configuration
use std::collections::HashMap;
use std::path::Path;

use bitcoin::{ScriptBuf, XOnlyPublicKey};
use clarity::types::chainstate::StacksAddress;
use clarity::vm::types::{PrincipalData, QualifiedContractIdentifier};
use config::{Config, Environment, File};
use serde::Deserialize;
use url::Url;

use crate::config::error::SpoxConfigError;
use crate::config::serialization::{
    contract_deserializer_option, duration_seconds_deserializer, principal_deserializer,
    script_deserializer, stacks_address_deserializer, url_deserializer, xonly_deserializer,
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
    /// The reclaim script
    #[serde(deserialize_with = "script_deserializer")]
    pub reclaim_script: ScriptBuf,
}

/// Top-level configuration
#[derive(Deserialize, Clone, Debug)]
pub struct Settings {
    /// Bitcoin RPC endpoint
    #[serde(deserialize_with = "url_deserializer")]
    pub bitcoin_rpc_endpoint: Url,
    /// Emily API endpoint
    #[serde(deserialize_with = "url_deserializer")]
    pub emily_endpoint: Url,
    /// How often looking for new deposit transactions
    #[serde(deserialize_with = "duration_seconds_deserializer")]
    pub polling_interval: std::time::Duration,
    /// HTTP request timeout for Bitcoin Core RPC calls. Must be larger than
    /// the worst-case `scantxoutset` duration on the target node.
    #[serde(deserialize_with = "duration_seconds_deserializer")]
    pub bitcoin_rpc_timeout: std::time::Duration,
    /// Monitored deposits
    #[serde(default)]
    pub deposit: HashMap<String, MonitoredDepositConfig>,
    /// Registry smart contract address
    #[serde(default, deserialize_with = "contract_deserializer_option")]
    pub registry_contract: Option<QualifiedContractIdentifier>,
    /// Stacks config, used only for some CLI commands and for the registry contract
    pub stacks: Option<StacksConfig>,
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

impl Settings {
    /// Initializing the global config first with default values and then with
    /// provided/overwritten environment variables. The explicit separator with
    /// double underscores is needed to correctly parse the nested config structure.
    ///
    /// The environment variables are prefixed with `SPOX_` and the nested
    /// fields are separated with double underscores.
    pub fn new(config_path: Option<impl AsRef<Path>>) -> Result<Self, SpoxConfigError> {
        // To properly parse lists from both environment and config files while
        // using a custom deserializer, we need to specify the list separator,
        // enable try_parsing and specify the keys which should be parsed as lists.
        // If the keys aren't specified, the deserializer will try to parse all
        // Strings as lists which will result in an error.
        let env = Environment::with_prefix(CONFIG_PREFIX)
            .prefix_separator("_")
            .separator("__")
            .try_parsing(true);

        let mut cfg_builder = Config::builder();

        cfg_builder = cfg_builder.set_default("polling_interval", 30)?;
        cfg_builder = cfg_builder.set_default("bitcoin_rpc_timeout", 15 * 60)?;

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

        Ok(())
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
        assert_eq!(settings.bitcoin_rpc_timeout, Duration::from_secs(15 * 60));
        assert!(settings.registry_contract.is_none());
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
}
