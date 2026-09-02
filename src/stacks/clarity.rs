//! Helper utilities for dealing with data returned from Clarity smart
//! contracts.

use std::collections::BTreeMap;

use clarity::vm::ClarityName;
use clarity::vm::Value as ClarityValue;
use clarity::vm::types::ListData;
use clarity::vm::types::OptionalData;
use clarity::vm::types::PrincipalData;
use clarity::vm::types::SequenceData;
use clarity::vm::types::TupleData;

use crate::error::Error;

/// A struct in a Clarity smart contract, which they call a tuple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClarityTuple(BTreeMap<ClarityName, ClarityValue>);

impl ClarityTuple {
    /// Create a new ClarityTuple from a BTreeMap of ClarityName to ClarityValue.
    pub fn new(data_map: BTreeMap<ClarityName, ClarityValue>) -> Self {
        Self(data_map)
    }

    /// Extract the buff value from the given field
    pub fn remove_buff(&mut self, field: &'static str) -> Result<Vec<u8>, Error> {
        match self.0.remove(field) {
            Some(ClarityValue::Sequence(SequenceData::Buffer(buf))) => Ok(buf.data),
            _ => Err(Error::ClarityMissingTupleEntry(field)),
        }
    }

    /// Extract the list value from the given field
    pub fn remove_list(&mut self, field: &'static str) -> Result<Vec<ClarityValue>, Error> {
        match self.0.remove(field) {
            Some(ClarityValue::Sequence(SequenceData::List(ListData { data, .. }))) => Ok(data),
            _ => Err(Error::ClarityMissingTupleEntry(field)),
        }
    }

    /// Extract the option value from the given field
    pub fn remove_option(&mut self, field: &'static str) -> Result<Option<ClarityValue>, Error> {
        match self.0.remove(field) {
            Some(ClarityValue::Optional(OptionalData { data })) => Ok(data.map(|x| *x)),
            _ => Err(Error::ClarityMissingTupleEntry(field)),
        }
    }

    /// Extract the principal value from the given field
    pub fn remove_principal(&mut self, field: &'static str) -> Result<PrincipalData, Error> {
        match self.0.remove(field) {
            Some(ClarityValue::Principal(data)) => Ok(data),
            _ => Err(Error::ClarityMissingTupleEntry(field)),
        }
    }

    /// Extract the u128 value from the given field
    pub fn remove_uint(&mut self, field: &'static str) -> Result<u128, Error> {
        match self.0.remove(field) {
            Some(ClarityValue::UInt(data)) => Ok(data),
            _ => Err(Error::ClarityMissingTupleEntry(field)),
        }
    }
}

impl TryFrom<ClarityValue> for ClarityTuple {
    type Error = Error;

    fn try_from(value: ClarityValue) -> Result<Self, Self::Error> {
        let ClarityValue::Tuple(TupleData { data_map, .. }) = value else {
            return Err(Error::InvalidStacksResponse("did not get a tuple"));
        };

        Ok(ClarityTuple::new(data_map))
    }
}
