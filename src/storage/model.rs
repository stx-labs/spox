//! Data models used in spox.

use bitcoin::ScriptBuf;
use clarity::vm::types::PrincipalData;
use sbtc::deposits::{DepositScriptInputs, ReclaimScriptInputs};
use serde::de::DeserializeOwned;

use crate::{config::MonitoredDepositConfig, error::Error};

/// The source for a deposit
#[derive(Debug, Clone, PartialEq)]
pub enum MonitoredDepositSource {
    /// Address is configured in the config file (with this alias)
    Config(String),
    /// Address is registered on the smart contract registry (with this ID)
    Registry(u64),
}

/// A deposit address to monitor
#[derive(Debug, Clone, PartialEq)]
pub struct MonitoredDeposit {
    /// Monitored deposit source
    pub source: MonitoredDepositSource,
    /// Deposit script inputs
    pub deposit_script_inputs: DepositScriptInputs,
    /// Reclaim script inputs
    pub reclaim_script_inputs: ReclaimScriptInputs,
}

impl MonitoredDeposit {
    /// Get the scriptPubKey for this deposit address
    pub fn to_script_pubkey(&self) -> ScriptBuf {
        sbtc::deposits::to_script_pubkey(
            self.deposit_script_inputs.deposit_script(),
            self.reclaim_script_inputs.reclaim_script(),
        )
    }
}

/// Convert a principal into the type expected by the sbtc crate.
pub fn as_principal_data<T>(principal: &PrincipalData) -> Result<T, Error>
where
    T: DeserializeOwned,
{
    let value = serde_json::to_value(principal).map_err(Error::PrincipalDataSerdeJson)?;
    serde_json::from_value(value).map_err(Error::PrincipalDataSerdeJson)
}

impl TryFrom<(&String, &MonitoredDepositConfig)> for MonitoredDeposit {
    type Error = Error;

    fn try_from((alias, deposit): (&String, &MonitoredDepositConfig)) -> Result<Self, Self::Error> {
        let deposit = deposit.clone();
        Ok(MonitoredDeposit {
            source: MonitoredDepositSource::Config(alias.clone()),
            deposit_script_inputs: DepositScriptInputs {
                signers_public_key: deposit.signers_xonly,
                recipient: as_principal_data(&deposit.recipient)?,
                max_fee: deposit.max_fee,
            },
            reclaim_script_inputs: ReclaimScriptInputs::try_new(
                deposit.lock_time,
                deposit.reclaim_script,
            )?,
        })
    }
}
