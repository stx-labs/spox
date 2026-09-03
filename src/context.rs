//! Application context

use std::sync::Arc;

use emily_client::apis::configuration::Configuration as EmilyConfig;

use crate::bitcoin::node::BitcoinCoreClient;
use crate::config::Settings;
use crate::error::Error;
use crate::stacks::node::StacksClient;
use crate::stacks::registry::DepositAddressRegistry;
use crate::stacks::wallet::StacksWallet;
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
        let bitcoin_client = BitcoinCoreClient::from_config(
            &value.bitcoin_rpc_endpoint,
            value.node_wallet.as_ref().map(|w| w.name.as_str()),
            value.bitcoin_rpc_timeout,
        )?;
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
    /// Construct a Stacks wallet given the information in the config.
    ///
    /// # Note
    ///
    /// This function reaches out to the stacks node to get the current
    /// chain ID.
    pub async fn wallet(&self) -> Result<StacksWallet, Error> {
        let config = &self.settings;
        let Some(claims) = config.reward_claims.as_ref() else {
            return Err(Error::MissingRewardClaimsConfig);
        };
        let Some(stacks) = config.stacks.as_ref() else {
            return Err(Error::MissingStacksConfig);
        };

        // Let's go and get the current chain id.
        let client = StacksClient::new(stacks.rpc_endpoint.clone())?;
        let info = client.get_node_info().await?;
        Ok(StacksWallet::new(claims.private_key, info.chain_id, 0))
    }

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
