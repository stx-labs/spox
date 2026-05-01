//! Application context

use std::sync::Arc;

use emily_client::apis::configuration::Configuration as EmilyConfig;

use crate::bitcoin::node::BitcoinCoreClient;
use crate::config::Settings;
use crate::error::Error;
use crate::stacks::node::StacksClient;
use crate::stacks::registry::DepositAddressRegistry;
use crate::storage::memory::{SharedStore, Store};

/// Application context
#[derive(Clone)]
pub struct Context {
    bitcoin_client: BitcoinCoreClient,
    emily_config: Arc<EmilyConfig>,
    storage: SharedStore,
    settings: Arc<Settings>,
    registry: Option<Arc<DepositAddressRegistry>>,
}

impl TryFrom<&Settings> for Context {
    type Error = Error;

    fn try_from(value: &Settings) -> Result<Self, Self::Error> {
        let bitcoin_client =
            BitcoinCoreClient::connect(&value.bitcoin_rpc_endpoint, value.bitcoin_rpc_timeout)?;
        let emily_config = EmilyConfig {
            base_path: value
                .emily_endpoint
                .to_string()
                .trim_end_matches('/')
                .to_string(),
            ..Default::default()
        };
        let registry = value
            .registry_contract
            .clone()
            .map(|registry_contract| {
                StacksClient::try_from(value)
                    .map(|client| Arc::new(DepositAddressRegistry::new(registry_contract, client)))
            })
            .transpose()?;

        Ok(Self {
            bitcoin_client,
            emily_config: Arc::new(emily_config),
            storage: Store::new_shared(),
            settings: Arc::new(value.clone()),
            registry,
        })
    }
}

impl Context {
    /// Get a reference to the Bitcoin client
    pub fn bitcoin_client(&self) -> &BitcoinCoreClient {
        &self.bitcoin_client
    }

    /// Get a reference to the Emily config
    pub fn emily_config(&self) -> &EmilyConfig {
        &self.emily_config
    }

    /// Get a reference to the storage
    pub fn storage(&self) -> SharedStore {
        self.storage.clone()
    }

    /// Get a reference to the config
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Get a reference to the registry
    pub fn registry(&self) -> Option<&DepositAddressRegistry> {
        self.registry.as_deref()
    }
}
