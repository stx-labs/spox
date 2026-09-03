//!  For constructing Stacks transactions that interact with the
//!  reward-claim registry.

use std::sync::LazyLock;

use blockstack_lib::chainstate::stacks::TransactionContractCall;
use blockstack_lib::chainstate::stacks::TransactionPayload;
use blockstack_lib::chainstate::stacks::TransactionPostCondition;
use blockstack_lib::chainstate::stacks::TransactionPostConditionMode;
use blockstack_lib::clarity::vm::ClarityName;
use blockstack_lib::clarity::vm::ContractName;
use blockstack_lib::clarity::vm::Value as ClarityValue;
use blockstack_lib::clarity::vm::types::ListData;
use blockstack_lib::clarity::vm::types::ListTypeData;
use blockstack_lib::clarity::vm::types::SequenceData;
use blockstack_lib::types::chainstate::StacksAddress;
use clarity::vm::types::CallableData;
use clarity::vm::types::QualifiedContractIdentifier;
use clarity::vm::types::StandardPrincipalData;
use clarity::vm::types::TraitIdentifier;
use clarity::vm::types::TypeSignature;

use crate::stacks::reward_claim_registry::MAX_STAKERS_LENGTH;
use crate::stacks::reward_claim_registry::RewardClaimsBatch;
use crate::stacks::reward_claim_registry::SettlementsBatch;
use crate::stacks::reward_claim_registry::TUPLE_SETTLEMENT_ITEM_SIGNATURE;

/// The type signature for the list of 100 stakers.
///
/// ListTypeData::new_list only returns an Err when max size of the type
/// could be greater than 1 MiB, or if the type depth is greater than 32.
/// Our type depth is 1 and the max size is 148 * 100 + 6 bytes, well under
/// 1 MiB. We also have a test that exercises this path so we know that
/// this won't panic in production.
static LIST_PRINCIPALS_SIGNATURE: LazyLock<ListTypeData> = LazyLock::new(|| {
    ListTypeData::new_list(TypeSignature::PrincipalType, MAX_STAKERS_LENGTH as u32).unwrap()
});

/// The type signature for the list of 100 `{staker, request-id}` items.
///
/// Same size bounds as [`LIST_PRINCIPALS_SIGNATURE`]: depth is small and the
/// max serialized size is well under 1 MiB. Exercised by the dummy
/// `SettlementsBatch` contract-call test.
static LIST_SETTLEMENT_ITEMS_SIGNATURE: LazyLock<ListTypeData> = LazyLock::new(|| {
    let entry_type = TUPLE_SETTLEMENT_ITEM_SIGNATURE.clone().into();
    ListTypeData::new_list(entry_type, MAX_STAKERS_LENGTH as u32).unwrap()
});

/// The name of the trait that signer managers must implement.
const REWARD_CLAIM_TRAIT_NAME: &str = "reward-claim-signer-manager-trait";

/// The name of the contract that contains the signer manager trait.
const REWARD_CLAIM_TRAIT_CONTRACT: &str = "reward-claim-traits";

/// This function creates a Trait identifier for the
/// [`REWARD_CLAIM_TRAIT_NAME`] trait, once we know the issuer/deployer.
fn make_trait_identifier(deployer: StacksAddress) -> TraitIdentifier {
    TraitIdentifier {
        name: ClarityName::from(REWARD_CLAIM_TRAIT_NAME),
        contract_identifier: QualifiedContractIdentifier {
            issuer: StandardPrincipalData::from(deployer),
            // The following From::from call is more dangerous than it
            // appears. Under the hood it calls its TryFrom::try_from
            // implementation and then unwrap them. We check that this
            // is fine in our test.
            name: ContractName::from(REWARD_CLAIM_TRAIT_CONTRACT),
        },
    }
}

/// A struct describing any transaction post-execution conditions that we'd
/// like to enforce.
///
/// # Note
///
/// * SIP-005 describes the post conditions, including its limitations, and
///   can be found here
///   https://github.com/stacksgov/sips/blob/main/sips/sip-005/sip-005-blocks-and-transactions.md#transaction-post-conditions
#[derive(Debug)]
pub struct StacksTxPostConditions {
    /// Specifies whether other asset transfers not covered by the
    /// post-conditions are permitted.
    pub post_condition_mode: TransactionPostConditionMode,
    /// Any post-execution conditions that we'd like to enforce.
    pub post_conditions: Vec<TransactionPostCondition>,
}

/// A trait to ease construction of a contract call StacksTransaction.
pub trait IntoContractCall: Sized {
    /// The name of the clarity smart contract that relates to this struct.
    const CONTRACT_NAME: &'static str;
    /// The specific function name that relates to this struct.
    const FUNCTION_NAME: &'static str;
    /// The stacks address that deployed the contract.
    fn deployer_address(&self) -> &StacksAddress;
    /// The arguments to the clarity function.
    fn into_contract_args(self) -> Vec<ClarityValue>;
    /// The payload of the transaction
    fn into_tx_payload(self) -> TransactionPayload {
        let contract_call = TransactionContractCall {
            address: self.deployer_address().clone(),
            // The following From::from calls are more dangerous than they
            // appear. Under the hood they call their TryFrom::try_from
            // implementation and then unwrap them(!). We check that this
            // is fine in our test.
            function_name: ClarityName::from(Self::FUNCTION_NAME),
            contract_name: ContractName::from(Self::CONTRACT_NAME),
            function_args: self.into_contract_args(),
        };
        TransactionPayload::ContractCall(contract_call)
    }
    /// Any post-execution conditions that we'd like to enforce. The
    /// deployer corresponds to the principal in the Transaction
    /// post-conditions, which is the address that sent the asset. The
    /// default is that we do not enforce any conditions since we usually
    /// deployed the contract.
    fn post_conditions(&self) -> StacksTxPostConditions {
        StacksTxPostConditions {
            post_condition_mode: TransactionPostConditionMode::Allow,
            post_conditions: Vec::new(),
        }
    }
}

impl IntoContractCall for RewardClaimsBatch {
    /// The name of the clarity smart contract that relates to this struct.
    const CONTRACT_NAME: &'static str = "reward-claim-registry";
    /// The specific function name that relates to this struct.
    const FUNCTION_NAME: &'static str = "process-reward-claims";
    /// The stacks address that deployed the contract.
    fn deployer_address(&self) -> &StacksAddress {
        self.deployer()
    }
    /// The arguments to the clarity function.
    fn into_contract_args(self) -> Vec<ClarityValue> {
        let callable = CallableData {
            contract_identifier: self.signer_manager().clone(),
            trait_identifier: Some(make_trait_identifier(self.deployer().clone())),
        };
        let stakers = self.stakers().into_iter().map(ClarityValue::Principal);
        let stakers = ListData {
            data: stakers.collect(),
            type_signature: LIST_PRINCIPALS_SIGNATURE.clone(),
        };

        vec![
            ClarityValue::CallableContract(callable),
            ClarityValue::Sequence(SequenceData::List(stakers)),
        ]
    }
}

impl IntoContractCall for SettlementsBatch {
    /// The name of the clarity smart contract that relates to this struct.
    const CONTRACT_NAME: &'static str = "reward-claim-registry";
    /// The specific function name that relates to this struct.
    const FUNCTION_NAME: &'static str = "settle-pending-withdrawals";
    /// The stacks address that deployed the contract.
    fn deployer_address(&self) -> &StacksAddress {
        self.deployer()
    }
    /// The arguments to the clarity function.
    fn into_contract_args(self) -> Vec<ClarityValue> {
        let callable = CallableData {
            contract_identifier: self.signer_manager().clone(),
            trait_identifier: Some(make_trait_identifier(self.deployer().clone())),
        };
        let data = self
            .into_items()
            .map(|item| ClarityValue::Tuple(item.into_tuple()))
            .collect::<Vec<_>>();
        let type_signature = LIST_SETTLEMENT_ITEMS_SIGNATURE.clone();

        vec![
            ClarityValue::CallableContract(callable),
            ClarityValue::Sequence(SequenceData::List(ListData { data, type_signature })),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let call = SettlementsBatch::dummy();

        let _ = call.into_tx_payload();
    }
}
