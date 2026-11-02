//! In-memory implementation of the Storage trait

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use bitcoin::ScriptBuf;

use crate::error::Error;
use crate::storage::Storage;
use crate::storage::model::MonitoredDeposit;

/// A store wrapped in an Arc<Mutex<...>> for interior mutability
pub type SharedStore = Arc<Mutex<Store>>;

/// In-memory store
#[derive(Debug, Default)]
pub struct Store {
    last_next_address_id: u64,
    monitored: HashMap<ScriptBuf, MonitoredDeposit>,
}

impl Store {
    /// Create an empty store wrapped in an Arc<Mutex<...>>
    pub fn new_shared() -> SharedStore {
        Arc::new(Mutex::new(Self::default()))
    }
}

/// Storage trait implementation for the in-memory store
impl Storage for SharedStore {
    /// Add a monitored deposit. If the script pubkey already exists it's a nop.
    fn add(&self, monitored_deposit: MonitoredDeposit) -> Result<(), Error> {
        let mut store = self.lock().map_err(|_| Error::PoisonedMutex)?;

        let key = monitored_deposit.to_script_pubkey();
        store.monitored.entry(key).or_insert(monitored_deposit);
        Ok(())
    }

    /// Get a monitored deposit by script pubkey.
    fn get_by_script(&self, script: &ScriptBuf) -> Result<Option<MonitoredDeposit>, Error> {
        let store = self.lock().map_err(|_| Error::PoisonedMutex)?;

        Ok(store.monitored.get(script).cloned())
    }

    /// Get the script pubkeys for monitored deposits
    fn get_scripts(&self) -> Result<Vec<ScriptBuf>, Error> {
        let store = self.lock().map_err(|_| Error::PoisonedMutex)?;

        Ok(store.monitored.keys().cloned().collect())
    }

    /// Get the last next-address-id from the registry
    fn get_last_next_address_id(&self) -> Result<u64, Error> {
        let store = self.lock().map_err(|_| Error::PoisonedMutex)?;

        Ok(store.last_next_address_id)
    }

    /// Set the last next-address-id from the registry
    fn set_last_next_address_id(&self, next_address_id: u64) -> Result<(), Error> {
        let mut store = self.lock().map_err(|_| Error::PoisonedMutex)?;

        store.last_next_address_id = next_address_id;
        Ok(())
    }
}
