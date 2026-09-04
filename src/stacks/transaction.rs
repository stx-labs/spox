//!  For constructing Stacks transactions that interact with the
//!  reward-claim registry.

use blockstack_lib::chainstate::stacks::TransactionContractCall;
use blockstack_lib::chainstate::stacks::TransactionPayload;
use blockstack_lib::clarity::vm::ClarityName;
use blockstack_lib::clarity::vm::Value as ClarityValue;
use clarity::vm::types::QualifiedContractIdentifier;
use clarity::vm::types::TraitIdentifier;

/// The name of the trait that signer managers must implement.
const REWARD_CLAIM_TRAIT_NAME: &str = "reward-claim-signer-manager-trait";

/// This function creates a Trait identifier for the
/// [`REWARD_CLAIM_TRAIT_NAME`] trait, once we know the issuer/deployer.
pub fn make_trait_identifier(contract: QualifiedContractIdentifier) -> Box<TraitIdentifier> {
    Box::new(TraitIdentifier {
        name: ClarityName::from_literal(REWARD_CLAIM_TRAIT_NAME),
        contract_identifier: contract,
    })
}

/// A trait to ease construction of a contract call StacksTransaction.
pub trait IntoContractCall: Sized {
    /// The specific function name that relates to this struct.
    const FUNCTION_NAME: &'static str;
    /// The identifier of the registry contract.
    fn contract_identifier(&self) -> &QualifiedContractIdentifier;
    /// The arguments to the clarity function.
    fn into_contract_args(self) -> Vec<ClarityValue>;
    /// The payload of the transaction
    fn into_tx_payload(self) -> TransactionPayload {
        let contract_identifier = self.contract_identifier().clone();
        let contract_call = TransactionContractCall {
            address: contract_identifier.issuer.into(),
            // The ContractName::from_literal call is more dangerous than
            // it appears. Under the hood it calls its TryFrom::try_from
            // implementation and then unwrap them. We check that this is
            // fine in our test.
            function_name: ClarityName::from_literal(Self::FUNCTION_NAME),
            contract_name: contract_identifier.name,
            function_args: self.into_contract_args(),
        };
        TransactionPayload::ContractCall(contract_call)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::stacks::reward_claim_registry::RewardClaimsBatch;
    use crate::stacks::reward_claim_registry::WithdrawalsBatch;

    #[test]
    fn process_reward_claims_contract_call_creation() {
        // This is to check that this function doesn't implicitly panic. If
        // it doesn't panic now, it can never panic at runtime.
        let call = RewardClaimsBatch::dummy();

        let _ = call.into_tx_payload();
    }

    #[test]
    fn settle_pending_withdrawals_contract_call_creation() {
        // This is to check that this function doesn't implicitly panic. If
        // it doesn't panic now, it can never panic at runtime.
        let call = WithdrawalsBatch::dummy();

        let _ = call.into_tx_payload();
    }
}
