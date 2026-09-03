//! Client for the on-chain reward claim registry.

use std::collections::HashMap;

use clarity::types::chainstate::StacksAddress;
use clarity::types::chainstate::StacksBlockId;
use clarity::vm::ClarityName;
use clarity::vm::ContractName;
use clarity::vm::Value as ClarityValue;
use clarity::vm::types::PrincipalData;
use clarity::vm::types::QualifiedContractIdentifier;
use clarity::vm::types::TupleData;

use crate::error::Error;
use crate::stacks::clarity::ClarityTuple;
use crate::stacks::node::StacksClient;

/// Maximum number of stakers accepted by `process-reward-claims` in one call.
pub const MAX_STAKERS_LENGTH: usize = 100;

/// Key identifying a registration in the reward claim registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationKey {
    /// The staker principal on the registration.
    pub staker: PrincipalData,
    /// The signer-manager principal on the registration.
    pub signer_manager: PrincipalData,
}

/// A single pending claim row from `get-pending-claims`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingClaim {
    /// The signer-manager principal for this registration.
    pub signer_manager: QualifiedContractIdentifier,
    /// The staker principal for this registration.
    pub staker: PrincipalData,
    /// Bond index when the staker is in a bond; `None` for STX-only stakes.
    pub bond_index: Option<u128>,
    /// The pox-5 reward cycle to claim.
    pub reward_cycle: u128,
}

impl PendingClaim {
    /// Registration key for this claim row.
    pub fn registration_key(&self) -> RegistrationKey {
        RegistrationKey {
            staker: self.staker.clone(),
            signer_manager: PrincipalData::Contract(self.signer_manager.clone()),
        }
    }
}

/// One page from `get-pending-claims`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingClaimsPage {
    /// Pending claim rows found during this bounded walk.
    pub claims: Vec<PendingClaim>,
    /// Cursor to pass to the next `get_pending_claims` call.
    ///
    /// `None` means the walk reached the tail of the registration list.
    /// `Some` is the last visited registration key, which may not be a
    /// pending claim itself.
    pub next: Option<RegistrationKey>,
}

/// Key identifying a pending withdrawal in the reward claim registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementKey {
    /// The staker principal on the pending withdrawal.
    pub staker: PrincipalData,
    /// The signer-manager principal on the pending withdrawal.
    pub signer_manager: PrincipalData,
    /// The sBTC withdrawal request ID.
    pub request_id: u128,
}

/// A single pending settlement row from `get-pending-settlements`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSettlement {
    /// The signer-manager principal for this pending withdrawal.
    pub signer_manager: QualifiedContractIdentifier,
    /// The staker principal for this pending withdrawal.
    pub staker: PrincipalData,
    /// The sBTC withdrawal request ID to settle.
    pub request_id: u128,
}

impl PendingSettlement {
    /// Settlement key for this row.
    pub fn settlement_key(&self) -> SettlementKey {
        SettlementKey {
            staker: self.staker.clone(),
            signer_manager: PrincipalData::Contract(self.signer_manager.clone()),
            request_id: self.request_id,
        }
    }
}

/// One page from `get-pending-settlements`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSettlementsPage {
    /// Pending settlement rows found during this bounded walk.
    pub settlements: Vec<PendingSettlement>,
    /// Cursor to pass to the next `get_pending_settlements` call.
    ///
    /// `None` means the walk reached the tail of the pending-withdrawal
    /// list. `Some` is the last visited settlement key, which may not be
    /// a settleable row itself.
    pub next: Option<SettlementKey>,
}

/// Arguments for one `process-reward-claims` contract call.
///
/// All [`Self::stakers`] share [`Self::signer_manager`], and the list
/// length is at most [`MAX_STAKERS_LENGTH`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewardClaimsBatch {
    /// Signer-manager trait principal passed to `process-reward-claims`.
    signer_manager: QualifiedContractIdentifier,
    /// Staker principals to claim for in this call (1..=100).
    stakers: Vec<PrincipalData>,
    /// The address that deployed the rewards claim registry.
    deployer: StacksAddress,
}

impl RewardClaimsBatch {
    /// Create a new dummy reward claims batch.
    #[cfg(test)]
    pub fn dummy() -> Self {
        Self {
            signer_manager: QualifiedContractIdentifier::transient(),
            stakers: vec![PrincipalData::from(StacksAddress::burn_address(false))],
            deployer: StacksAddress::burn_address(false),
        }
    }

    /// The signer-manager principal passed to `process-reward-claims`.
    pub fn signer_manager(&self) -> &QualifiedContractIdentifier {
        &self.signer_manager
    }

    /// The stakers to claim for in this call.
    pub fn stakers(&self) -> &[PrincipalData] {
        &self.stakers
    }

    /// The address that deployed the rewards claim registry.
    pub fn deployer(&self) -> &StacksAddress {
        &self.deployer
    }
}

/// Client for querying the on-chain reward claim registry contract.
#[derive(Debug, Clone)]
pub struct RewardClaimRegistry {
    /// The deployer of the registry smart contract.
    deployer: StacksAddress,
    /// The name of the registry smart contract.
    contract_name: ContractName,
    /// The client used to make the requests.
    client: StacksClient,
}

impl RewardClaimRegistry {
    /// Create a new reward claim registry client.
    pub fn new(contract: QualifiedContractIdentifier, client: StacksClient) -> Self {
        let deployer = contract.issuer.into();

        Self {
            contract_name: contract.name,
            deployer,
            client,
        }
    }

    /// Get a reference to the client.
    pub fn client(&self) -> &StacksClient {
        &self.client
    }

    /// Fetch a page of pending claims from the registry.
    ///
    /// Pass `None` for `cursor` to start at the head of the registration
    /// linked list. To paginate, pass [`PendingClaimsPage::next`] from the
    /// previous page. `next == None` means the walk reached the tail; an
    /// empty `claims` list alone does not.
    async fn get_pending_claims(
        &self,
        cursor: Option<&RegistrationKey>,
        chain_tip: Option<&StacksBlockId>,
    ) -> Result<PendingClaimsPage, Error> {
        let cursor_arg = match cursor {
            Some(key) => {
                let tuple = TupleData::from_data(vec![
                    (
                        ClarityName::from("staker"),
                        ClarityValue::Principal(key.staker.clone()),
                    ),
                    (
                        ClarityName::from("signer-manager"),
                        ClarityValue::Principal(key.signer_manager.clone()),
                    ),
                ])
                .map_err(|_| Error::InvalidStacksResponse("could not construct cursor tuple"))?;
                ClarityValue::some(ClarityValue::Tuple(tuple)).map_err(|_| {
                    Error::InvalidStacksResponse("could not construct cursor option")
                })?
            }
            None => ClarityValue::none(),
        };

        let result = self
            .client
            .call_read(
                &self.deployer,
                &self.contract_name,
                &ClarityName::from("get-pending-claims"),
                &self.deployer,
                &[cursor_arg],
                chain_tip,
            )
            .await?;

        let ClarityValue::Response(response) = result else {
            return Err(Error::InvalidStacksResponse("expected a response"));
        };

        PendingClaimsPage::try_from(*response.data)
    }

    /// Fetch every pending claim by paging through `get-pending-claims`.
    ///
    /// Continues while the page's `next` cursor is `Some`.
    async fn get_all_pending_claims(&self) -> Result<Vec<PendingClaim>, Error> {
        let mut all = Vec::new();
        let mut cursor: Option<RegistrationKey> = None;

        let tip = self.client.get_node_info().await?.chain_tip();
        let chain_tip = Some(&tip);

        loop {
            let page = self.get_pending_claims(cursor.as_ref(), chain_tip).await?;
            all.extend(page.claims);
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        Ok(all)
    }

    /// Get all pending claims, batched by signer manager, with chunks of
    /// at most [`MAX_STAKERS_LENGTH`] stakers per batch.
    pub async fn get_pending_claim_batches(&self) -> Result<Vec<RewardClaimsBatch>, Error> {
        let claims = self.get_all_pending_claims().await?;
        Ok(batch_claims(claims, &self.deployer))
    }

    /// Fetch a page of pending settlements from the registry.
    ///
    /// Pass `None` for `cursor` to start at the head of the pending-
    /// withdrawal linked list. To paginate, pass
    /// [`PendingSettlementsPage::next`] from the previous page.
    /// `next == None` means the walk reached the tail; an empty
    /// `settlements` list alone does not.
    async fn get_pending_settlements(
        &self,
        cursor: Option<&SettlementKey>,
        chain_tip: Option<&StacksBlockId>,
    ) -> Result<PendingSettlementsPage, Error> {
        let cursor_arg = match cursor {
            Some(key) => {
                let tuple = TupleData::from_data(vec![
                    (
                        ClarityName::from("staker"),
                        ClarityValue::Principal(key.staker.clone()),
                    ),
                    (
                        ClarityName::from("signer-manager"),
                        ClarityValue::Principal(key.signer_manager.clone()),
                    ),
                    (
                        ClarityName::from("request-id"),
                        ClarityValue::UInt(key.request_id),
                    ),
                ])
                .map_err(|_| Error::InvalidStacksResponse("could not construct cursor tuple"))?;
                ClarityValue::some(ClarityValue::Tuple(tuple)).map_err(|_| {
                    Error::InvalidStacksResponse("could not construct cursor option")
                })?
            }
            None => ClarityValue::none(),
        };

        let result = self
            .client
            .call_read(
                &self.deployer,
                &self.contract_name,
                &ClarityName::from("get-pending-settlements"),
                &self.deployer,
                &[cursor_arg],
                chain_tip,
            )
            .await?;

        let ClarityValue::Response(response) = result else {
            return Err(Error::InvalidStacksResponse("expected a response"));
        };

        PendingSettlementsPage::try_from(*response.data)
    }

    /// Fetch every pending settlement by paging through
    /// `get-pending-settlements`.
    ///
    /// Continues while the page's `next` cursor is `Some`.
    pub async fn get_all_pending_settlements(&self) -> Result<Vec<PendingSettlement>, Error> {
        let mut all = Vec::new();
        let mut cursor: Option<SettlementKey> = None;

        let tip = self.client.get_node_info().await?.chain_tip();
        let chain_tip = Some(&tip);

        loop {
            let page = self
                .get_pending_settlements(cursor.as_ref(), chain_tip)
                .await?;
            all.extend(page.settlements);
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        Ok(all)
    }
}

/// Group pending claims by signer-manager and split into contract-call batches.
///
/// Each batch has at most [`MAX_STAKERS_LENGTH`] stakers.
fn batch_claims(claims: Vec<PendingClaim>, deployer: &StacksAddress) -> Vec<RewardClaimsBatch> {
    let mut groups: HashMap<QualifiedContractIdentifier, Vec<PrincipalData>> = HashMap::new();

    for claim in claims {
        groups
            .entry(claim.signer_manager)
            .or_default()
            .push(claim.staker);
    }

    let mut batches = Vec::new();
    for (signer_manager, stakers) in groups {
        for chunk in stakers.chunks(MAX_STAKERS_LENGTH) {
            batches.push(RewardClaimsBatch {
                signer_manager: signer_manager.clone(),
                stakers: chunk.to_vec(),
                deployer: deployer.clone(),
            });
        }
    }

    batches
}

impl TryFrom<ClarityValue> for RegistrationKey {
    type Error = Error;

    fn try_from(value: ClarityValue) -> Result<Self, Self::Error> {
        let mut clarity_map = ClarityTuple::try_from(value)?;
        Ok(RegistrationKey {
            staker: clarity_map.remove_principal("staker")?,
            signer_manager: clarity_map.remove_principal("signer-manager")?,
        })
    }
}

impl TryFrom<ClarityValue> for PendingClaim {
    type Error = Error;

    fn try_from(value: ClarityValue) -> Result<Self, Self::Error> {
        let mut clarity_map = ClarityTuple::try_from(value)?;

        let bond_index = clarity_map
            .remove_option("bond-index")?
            .map(|value| match value {
                ClarityValue::UInt(index) => Ok(index),
                _ => Err(Error::InvalidStacksResponse("bond-index was not a uint")),
            })
            .transpose()?;

        let signer_manager = clarity_map.remove_principal("signer-manager")?;
        let PrincipalData::Contract(signer_manager) = signer_manager else {
            // This should never happen, because registration checks that
            // the signer manager implements a trait and only smart
            // contract principals can implement traits.
            return Err(Error::UnexpectedPrincipal(signer_manager));
        };

        Ok(PendingClaim {
            signer_manager,
            staker: clarity_map.remove_principal("staker")?,
            bond_index,
            reward_cycle: clarity_map.remove_uint("reward-cycle")?,
        })
    }
}

impl TryFrom<ClarityValue> for PendingClaimsPage {
    type Error = Error;

    fn try_from(value: ClarityValue) -> Result<Self, Self::Error> {
        let mut clarity_map = ClarityTuple::try_from(value)?;

        let claims = clarity_map
            .remove_list("rows")?
            .into_iter()
            .map(PendingClaim::try_from)
            .collect::<Result<Vec<_>, _>>()?;

        let next = clarity_map
            .remove_option("next")?
            .map(RegistrationKey::try_from)
            .transpose()?;

        Ok(PendingClaimsPage { claims, next })
    }
}

impl TryFrom<ClarityValue> for SettlementKey {
    type Error = Error;

    fn try_from(value: ClarityValue) -> Result<Self, Self::Error> {
        let mut clarity_map = ClarityTuple::try_from(value)?;
        Ok(SettlementKey {
            staker: clarity_map.remove_principal("staker")?,
            signer_manager: clarity_map.remove_principal("signer-manager")?,
            request_id: clarity_map.remove_uint("request-id")?,
        })
    }
}

impl TryFrom<ClarityValue> for PendingSettlement {
    type Error = Error;

    fn try_from(value: ClarityValue) -> Result<Self, Self::Error> {
        let mut clarity_map = ClarityTuple::try_from(value)?;

        let signer_manager = clarity_map.remove_principal("signer-manager")?;
        let PrincipalData::Contract(signer_manager) = signer_manager else {
            // This should never happen, because registration checks that
            // the signer manager implements a trait and only smart
            // contract principals can implement traits.
            return Err(Error::UnexpectedPrincipal(signer_manager));
        };

        Ok(PendingSettlement {
            signer_manager,
            staker: clarity_map.remove_principal("staker")?,
            request_id: clarity_map.remove_uint("request-id")?,
        })
    }
}

impl TryFrom<ClarityValue> for PendingSettlementsPage {
    type Error = Error;

    fn try_from(value: ClarityValue) -> Result<Self, Self::Error> {
        let mut clarity_map = ClarityTuple::try_from(value)?;

        let settlements = clarity_map
            .remove_list("rows")?
            .into_iter()
            .map(PendingSettlement::try_from)
            .collect::<Result<Vec<_>, _>>()?;

        let next = clarity_map
            .remove_option("next")?
            .map(SettlementKey::try_from)
            .transpose()?;

        Ok(PendingSettlementsPage { settlements, next })
    }
}

#[cfg(test)]
mod tests {
    use bitcoincore_rpc::jsonrpc::serde_json;
    use clarity::types::chainstate::BlockHeaderHash;
    use clarity::types::chainstate::ConsensusHash;
    use clarity::vm::types::OptionalData;

    use super::*;

    impl From<&PendingClaim> for ClarityValue {
        fn from(value: &PendingClaim) -> Self {
            let bond_index = value
                .bond_index
                .map(|index| Box::new(ClarityValue::UInt(index)));
            let tuple_entries = vec![
                (
                    ClarityName::from("signer-manager"),
                    ClarityValue::Principal(PrincipalData::Contract(value.signer_manager.clone())),
                ),
                (
                    ClarityName::from("staker"),
                    ClarityValue::Principal(value.staker.clone()),
                ),
                (
                    ClarityName::from("bond-index"),
                    ClarityValue::Optional(OptionalData { data: bond_index }),
                ),
                (
                    ClarityName::from("reward-cycle"),
                    ClarityValue::UInt(value.reward_cycle),
                ),
            ];
            ClarityValue::Tuple(TupleData::from_data(tuple_entries).unwrap())
        }
    }

    impl From<&RegistrationKey> for ClarityValue {
        fn from(value: &RegistrationKey) -> Self {
            let tuple_entries = vec![
                (
                    ClarityName::from("staker"),
                    ClarityValue::Principal(value.staker.clone()),
                ),
                (
                    ClarityName::from("signer-manager"),
                    ClarityValue::Principal(value.signer_manager.clone()),
                ),
            ];
            ClarityValue::Tuple(TupleData::from_data(tuple_entries).unwrap())
        }
    }

    impl From<&PendingSettlement> for ClarityValue {
        fn from(value: &PendingSettlement) -> Self {
            let tuple_entries = vec![
                (
                    ClarityName::from("signer-manager"),
                    ClarityValue::Principal(PrincipalData::Contract(value.signer_manager.clone())),
                ),
                (
                    ClarityName::from("staker"),
                    ClarityValue::Principal(value.staker.clone()),
                ),
                (
                    ClarityName::from("request-id"),
                    ClarityValue::UInt(value.request_id),
                ),
            ];
            ClarityValue::Tuple(TupleData::from_data(tuple_entries).unwrap())
        }
    }

    impl From<&SettlementKey> for ClarityValue {
        fn from(value: &SettlementKey) -> Self {
            let tuple_entries = vec![
                (
                    ClarityName::from("staker"),
                    ClarityValue::Principal(value.staker.clone()),
                ),
                (
                    ClarityName::from("signer-manager"),
                    ClarityValue::Principal(value.signer_manager.clone()),
                ),
                (
                    ClarityName::from("request-id"),
                    ClarityValue::UInt(value.request_id),
                ),
            ];
            ClarityValue::Tuple(TupleData::from_data(tuple_entries).unwrap())
        }
    }

    fn ok_page(claims: &[PendingClaim], next: Option<&RegistrationKey>) -> ClarityValue {
        let rows: Vec<ClarityValue> = claims.iter().map(ClarityValue::from).collect();
        let next_value = match next {
            Some(key) => ClarityValue::some(ClarityValue::from(key)).unwrap(),
            None => ClarityValue::none(),
        };
        let page = ClarityValue::Tuple(
            TupleData::from_data(vec![
                (
                    ClarityName::from("rows"),
                    ClarityValue::cons_list_unsanitized(rows).unwrap(),
                ),
                (ClarityName::from("next"), next_value),
            ])
            .unwrap(),
        );
        ClarityValue::okay(page).unwrap()
    }

    fn ok_settlements_page(
        settlements: &[PendingSettlement],
        next: Option<&SettlementKey>,
    ) -> ClarityValue {
        let rows: Vec<ClarityValue> = settlements.iter().map(ClarityValue::from).collect();
        let next_value = match next {
            Some(key) => ClarityValue::some(ClarityValue::from(key)).unwrap(),
            None => ClarityValue::none(),
        };
        let page = ClarityValue::Tuple(
            TupleData::from_data(vec![
                (
                    ClarityName::from("rows"),
                    ClarityValue::cons_list_unsanitized(rows).unwrap(),
                ),
                (ClarityName::from("next"), next_value),
            ])
            .unwrap(),
        );
        ClarityValue::okay(page).unwrap()
    }

    fn claim(
        signer_manager: &QualifiedContractIdentifier,
        staker: PrincipalData,
        reward_cycle: u128,
    ) -> PendingClaim {
        PendingClaim {
            signer_manager: signer_manager.clone(),
            staker,
            bond_index: None,
            reward_cycle,
        }
    }

    #[tokio::test]
    async fn get_pending_claims_works_without_cursor() {
        let staker = PrincipalData::parse("ST2FQWJMF9CGPW34ZWK8FEPNK072NEV1VKRNBBMJ9").unwrap();
        let signer_manager = QualifiedContractIdentifier::parse(
            "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.signer-manager",
        )
        .unwrap();

        let claim = PendingClaim {
            signer_manager,
            staker: staker.clone(),
            bond_index: None,
            reward_cycle: 42,
        };
        let next = claim.registration_key();

        let raw_json_response = format!(
            r#"{{"okay": true, "result":"0x{}"}}"#,
            ok_page(&[claim.clone()], Some(&next))
                .serialize_to_hex()
                .unwrap(),
        );

        let mut stacks_node_server = mockito::Server::new_async().await;
        let mock = stacks_node_server
            .mock(
                "POST",
                "/v2/contracts/call-read/ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039/reward-claim-registry/get-pending-claims?tip=latest",
            )
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "arguments": [ClarityValue::none().serialize_to_hex().unwrap()]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&raw_json_response)
            .expect(1)
            .create();

        let client_url = url::Url::parse(stacks_node_server.url().as_str()).unwrap();
        let client = StacksClient::new(client_url).unwrap();

        let registry = RewardClaimRegistry::new(
            QualifiedContractIdentifier::parse(
                "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.reward-claim-registry",
            )
            .unwrap(),
            client,
        );

        let result = registry.get_pending_claims(None, None).await.unwrap();

        assert_eq!(
            result,
            PendingClaimsPage {
                claims: vec![claim],
                next: Some(next),
            }
        );
        mock.assert();
    }

    #[tokio::test]
    async fn get_pending_claims_works_with_cursor() {
        let staker = PrincipalData::parse("ST2FQWJMF9CGPW34ZWK8FEPNK072NEV1VKRNBBMJ9").unwrap();
        let signer_manager = QualifiedContractIdentifier::parse(
            "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.signer-manager",
        )
        .unwrap();

        let cursor = RegistrationKey {
            staker: staker.clone(),
            signer_manager: PrincipalData::Contract(signer_manager.clone()),
        };

        let claim = PendingClaim {
            signer_manager,
            staker: PrincipalData::parse("ST1PQHQKV0RJXZFY1DGX8MNSNYVE3VGZJSRTPGZGM").unwrap(),
            bond_index: Some(7),
            reward_cycle: 99,
        };

        let cursor_hex = ClarityValue::some(ClarityValue::from(&cursor))
            .unwrap()
            .serialize_to_hex()
            .unwrap();

        let raw_json_response = format!(
            r#"{{"okay": true, "result":"0x{}"}}"#,
            ok_page(&[claim.clone()], None).serialize_to_hex().unwrap(),
        );

        let mut stacks_node_server = mockito::Server::new_async().await;
        let mock = stacks_node_server
            .mock(
                "POST",
                "/v2/contracts/call-read/ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039/reward-claim-registry/get-pending-claims?tip=latest",
            )
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "arguments": [cursor_hex]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&raw_json_response)
            .expect(1)
            .create();

        let client_url = url::Url::parse(stacks_node_server.url().as_str()).unwrap();
        let client = StacksClient::new(client_url).unwrap();

        let registry = RewardClaimRegistry::new(
            QualifiedContractIdentifier::parse(
                "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.reward-claim-registry",
            )
            .unwrap(),
            client,
        );

        let result = registry
            .get_pending_claims(Some(&cursor), None)
            .await
            .unwrap();

        assert_eq!(
            result,
            PendingClaimsPage {
                claims: vec![claim],
                next: None,
            }
        );
        mock.assert();
    }

    #[tokio::test]
    async fn get_pending_claims_empty_page_at_tail() {
        let raw_json_response = format!(
            r#"{{"okay": true, "result":"0x{}"}}"#,
            ok_page(&[], None).serialize_to_hex().unwrap(),
        );

        let mut stacks_node_server = mockito::Server::new_async().await;
        let mock = stacks_node_server
            .mock(
                "POST",
                "/v2/contracts/call-read/ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039/reward-claim-registry/get-pending-claims?tip=latest",
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&raw_json_response)
            .expect(1)
            .create();

        let client_url = url::Url::parse(stacks_node_server.url().as_str()).unwrap();
        let client = StacksClient::new(client_url).unwrap();

        let registry = RewardClaimRegistry::new(
            QualifiedContractIdentifier::parse(
                "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.reward-claim-registry",
            )
            .unwrap(),
            client,
        );

        let result = registry.get_pending_claims(None, None).await.unwrap();

        assert_eq!(result, PendingClaimsPage { claims: vec![], next: None });
        mock.assert();
    }

    #[tokio::test]
    async fn get_all_pending_claims_continues_on_empty_rows_with_next() {
        let signer_manager = QualifiedContractIdentifier::parse(
            "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.signer-manager",
        )
        .unwrap();

        // First page: ticks burned on non-pending nodes, resume cursor only.
        let skipped = RegistrationKey {
            staker: PrincipalData::parse("ST2FQWJMF9CGPW34ZWK8FEPNK072NEV1VKRNBBMJ9").unwrap(),
            signer_manager: PrincipalData::Contract(signer_manager.clone()),
        };
        let pending = PendingClaim {
            signer_manager: signer_manager.clone(),
            staker: PrincipalData::parse("ST1PQHQKV0RJXZFY1DGX8MNSNYVE3VGZJSRTPGZGM").unwrap(),
            bond_index: Some(7),
            reward_cycle: 99,
        };

        let skipped_hex = ClarityValue::some(ClarityValue::from(&skipped))
            .unwrap()
            .serialize_to_hex()
            .unwrap();

        let stacks_tip = "b5f9aa4423ffa7abb585fc00e2783c40225597ec112ee618db86ae23dbbbe88c";
        let stacks_tip_consensus_hash = "dfe87cfd31c1a67fa8b989c83b79aa476e616758";
        let tip = StacksBlockId::new(
            &ConsensusHash::from_hex(stacks_tip_consensus_hash).unwrap(),
            &BlockHeaderHash::from_hex(stacks_tip).unwrap(),
        );

        let mut stacks_node_server = mockito::Server::new_async().await;
        let info_mock = stacks_node_server
            .mock("GET", "/v2/info")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(
                r#"{{
                    "network_id": 2147483648,
                    "stacks_tip": "{stacks_tip}",
                    "stacks_tip_consensus_hash": "{stacks_tip_consensus_hash}"
                }}"#
            ))
            .expect(1)
            .create();

        let path = format!(
            "/v2/contracts/call-read/ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039/reward-claim-registry/get-pending-claims?tip={tip}"
        );

        let empty_with_next = stacks_node_server
            .mock("POST", path.as_str())
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "arguments": [ClarityValue::none().serialize_to_hex().unwrap()]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(
                r#"{{"okay": true, "result":"0x{}"}}"#,
                ok_page(&[], Some(&skipped)).serialize_to_hex().unwrap(),
            ))
            .expect(1)
            .create();

        let pending_then_done = stacks_node_server
            .mock("POST", path.as_str())
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "arguments": [skipped_hex]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(
                r#"{{"okay": true, "result":"0x{}"}}"#,
                ok_page(&[pending.clone()], None)
                    .serialize_to_hex()
                    .unwrap(),
            ))
            .expect(1)
            .create();

        let client_url = url::Url::parse(stacks_node_server.url().as_str()).unwrap();
        let client = StacksClient::new(client_url).unwrap();

        let registry = RewardClaimRegistry::new(
            QualifiedContractIdentifier::parse(
                "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.reward-claim-registry",
            )
            .unwrap(),
            client,
        );

        let result = registry.get_all_pending_claims().await.unwrap();

        assert_eq!(result, vec![pending]);
        info_mock.assert();
        empty_with_next.assert();
        pending_then_done.assert();
    }

    #[test]
    fn batch_pending_claims_empty() {
        assert!(batch_claims(vec![], &StacksAddress::burn_address(false)).is_empty());
    }

    #[test]
    fn batch_pending_claims_groups_by_signer_manager() {
        let sm_a = QualifiedContractIdentifier::parse(
            "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.signer-manager",
        )
        .unwrap();
        let sm_b = QualifiedContractIdentifier::parse(
            "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.other-manager",
        )
        .unwrap();
        let staker1 = PrincipalData::parse("ST2FQWJMF9CGPW34ZWK8FEPNK072NEV1VKRNBBMJ9").unwrap();
        let staker2 = PrincipalData::parse("ST1PQHQKV0RJXZFY1DGX8MNSNYVE3VGZJSRTPGZGM").unwrap();
        let staker3 =
            PrincipalData::parse("ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.staker-3").unwrap();

        let claims = vec![
            claim(&sm_a, staker1.clone(), 1),
            claim(&sm_b, staker2.clone(), 2),
            claim(&sm_a, staker3.clone(), 3),
        ];

        let batches = batch_claims(claims, &StacksAddress::burn_address(false));
        assert_eq!(batches.len(), 2);

        let batch_a = batches
            .iter()
            .find(|batch| batch.signer_manager() == &sm_a)
            .expect("batch for signer-manager A");
        let batch_b = batches
            .iter()
            .find(|batch| batch.signer_manager() == &sm_b)
            .expect("batch for signer-manager B");

        assert_eq!(batch_a.stakers(), &[staker1, staker3]);
        assert_eq!(batch_b.stakers(), &[staker2]);
        assert_eq!(batch_a.deployer(), &StacksAddress::burn_address(false));
        assert_eq!(batch_b.deployer(), &StacksAddress::burn_address(false));
    }

    #[test]
    fn batch_pending_claims_chunks_at_max_stakers() {
        let sm = QualifiedContractIdentifier::parse(
            "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.signer-manager",
        )
        .unwrap();
        let claims: Vec<_> = (0..=MAX_STAKERS_LENGTH)
            .map(|i| {
                claim(
                    &sm,
                    PrincipalData::parse(&format!(
                        "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.s{i}"
                    ))
                    .unwrap(),
                    i as u128,
                )
            })
            .collect();

        let batches = batch_claims(claims.clone(), &StacksAddress::burn_address(false));
        assert_eq!(batches.len(), 2);
        assert!(batches.iter().all(|batch| batch.signer_manager() == &sm));
        assert_eq!(batches[0].stakers().len(), MAX_STAKERS_LENGTH);
        assert_eq!(batches[1].stakers().len(), 1);
        assert_eq!(batches[0].stakers()[0], claims[0].staker);
        assert_eq!(
            batches[0].stakers()[MAX_STAKERS_LENGTH - 1],
            claims[MAX_STAKERS_LENGTH - 1].staker
        );
        assert_eq!(batches[1].stakers()[0], claims[MAX_STAKERS_LENGTH].staker);
    }

    #[tokio::test]
    async fn get_pending_settlements_works_without_cursor() {
        let staker = PrincipalData::parse("ST2FQWJMF9CGPW34ZWK8FEPNK072NEV1VKRNBBMJ9").unwrap();
        let signer_manager = QualifiedContractIdentifier::parse(
            "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.signer-manager",
        )
        .unwrap();

        let settlement = PendingSettlement {
            signer_manager,
            staker: staker.clone(),
            request_id: 7,
        };
        let next = settlement.settlement_key();

        let raw_json_response = format!(
            r#"{{"okay": true, "result":"0x{}"}}"#,
            ok_settlements_page(&[settlement.clone()], Some(&next))
                .serialize_to_hex()
                .unwrap(),
        );

        let path = "/v2/contracts/call-read/ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039/reward-claim-registry/get-pending-settlements?tip=latest";
        let mut stacks_node_server = mockito::Server::new_async().await;
        let mock = stacks_node_server
            .mock("POST", path)
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "arguments": [ClarityValue::none().serialize_to_hex().unwrap()]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&raw_json_response)
            .expect(1)
            .create();

        let client_url = url::Url::parse(stacks_node_server.url().as_str()).unwrap();
        let client = StacksClient::new(client_url).unwrap();

        let registry = RewardClaimRegistry::new(
            QualifiedContractIdentifier::parse(
                "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.reward-claim-registry",
            )
            .unwrap(),
            client,
        );

        let result = registry.get_pending_settlements(None, None).await.unwrap();
        let expected = PendingSettlementsPage {
            settlements: vec![settlement],
            next: Some(next),
        };
        assert_eq!(result, expected);
        mock.assert();
    }

    #[tokio::test]
    async fn get_pending_settlements_works_with_cursor() {
        let staker = PrincipalData::parse("ST2FQWJMF9CGPW34ZWK8FEPNK072NEV1VKRNBBMJ9").unwrap();
        let signer_manager = QualifiedContractIdentifier::parse(
            "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.signer-manager",
        )
        .unwrap();

        let cursor = SettlementKey {
            staker: staker.clone(),
            signer_manager: PrincipalData::Contract(signer_manager.clone()),
            request_id: 1,
        };

        let settlement = PendingSettlement {
            signer_manager,
            staker: PrincipalData::parse("ST1PQHQKV0RJXZFY1DGX8MNSNYVE3VGZJSRTPGZGM").unwrap(),
            request_id: 99,
        };

        let cursor_hex = ClarityValue::some(ClarityValue::from(&cursor))
            .unwrap()
            .serialize_to_hex()
            .unwrap();

        let raw_json_response = format!(
            r#"{{"okay": true, "result":"0x{}"}}"#,
            ok_settlements_page(&[settlement.clone()], None)
                .serialize_to_hex()
                .unwrap(),
        );

        let mut stacks_node_server = mockito::Server::new_async().await;
        let mock = stacks_node_server
            .mock(
                "POST",
                "/v2/contracts/call-read/ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039/reward-claim-registry/get-pending-settlements?tip=latest",
            )
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "arguments": [cursor_hex]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&raw_json_response)
            .expect(1)
            .create();

        let client_url = url::Url::parse(stacks_node_server.url().as_str()).unwrap();
        let client = StacksClient::new(client_url).unwrap();

        let registry = RewardClaimRegistry::new(
            QualifiedContractIdentifier::parse(
                "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.reward-claim-registry",
            )
            .unwrap(),
            client,
        );

        let result = registry
            .get_pending_settlements(Some(&cursor), None)
            .await
            .unwrap();

        let expected = PendingSettlementsPage {
            settlements: vec![settlement],
            next: None,
        };
        assert_eq!(result, expected);
        mock.assert();
    }

    #[tokio::test]
    async fn get_pending_settlements_empty_page_at_tail() {
        let raw_json_response = format!(
            r#"{{"okay": true, "result":"0x{}"}}"#,
            ok_settlements_page(&[], None).serialize_to_hex().unwrap(),
        );

        let mut stacks_node_server = mockito::Server::new_async().await;
        let mock = stacks_node_server
            .mock(
                "POST",
                "/v2/contracts/call-read/ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039/reward-claim-registry/get-pending-settlements?tip=latest",
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&raw_json_response)
            .expect(1)
            .create();

        let client_url = url::Url::parse(stacks_node_server.url().as_str()).unwrap();
        let client = StacksClient::new(client_url).unwrap();

        let registry = RewardClaimRegistry::new(
            QualifiedContractIdentifier::parse(
                "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.reward-claim-registry",
            )
            .unwrap(),
            client,
        );

        let result = registry.get_pending_settlements(None, None).await.unwrap();

        let expected = PendingSettlementsPage {
            settlements: vec![],
            next: None,
        };
        assert_eq!(result, expected);
        mock.assert();
    }

    #[tokio::test]
    async fn get_all_pending_settlements_continues_on_empty_rows_with_next() {
        let signer_manager = QualifiedContractIdentifier::parse(
            "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.signer-manager",
        )
        .unwrap();

        // First page: ticks burned on non-settleable nodes, resume cursor only.
        let skipped = SettlementKey {
            staker: PrincipalData::parse("ST2FQWJMF9CGPW34ZWK8FEPNK072NEV1VKRNBBMJ9").unwrap(),
            signer_manager: PrincipalData::Contract(signer_manager.clone()),
            request_id: 3,
        };
        let pending = PendingSettlement {
            signer_manager: signer_manager.clone(),
            staker: PrincipalData::parse("ST1PQHQKV0RJXZFY1DGX8MNSNYVE3VGZJSRTPGZGM").unwrap(),
            request_id: 99,
        };

        let skipped_hex = ClarityValue::some(ClarityValue::from(&skipped))
            .unwrap()
            .serialize_to_hex()
            .unwrap();

        let stacks_tip = "b5f9aa4423ffa7abb585fc00e2783c40225597ec112ee618db86ae23dbbbe88c";
        let stacks_tip_consensus_hash = "dfe87cfd31c1a67fa8b989c83b79aa476e616758";
        let tip = StacksBlockId::new(
            &ConsensusHash::from_hex(stacks_tip_consensus_hash).unwrap(),
            &BlockHeaderHash::from_hex(stacks_tip).unwrap(),
        );

        let mut stacks_node_server = mockito::Server::new_async().await;
        let info_mock = stacks_node_server
            .mock("GET", "/v2/info")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(
                r#"{{
                    "network_id": 2147483648,
                    "stacks_tip": "{stacks_tip}",
                    "stacks_tip_consensus_hash": "{stacks_tip_consensus_hash}"
                }}"#
            ))
            .expect(1)
            .create();

        let path = format!(
            "/v2/contracts/call-read/ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039/reward-claim-registry/get-pending-settlements?tip={tip}"
        );

        let empty_with_next = stacks_node_server
            .mock("POST", path.as_str())
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "arguments": [ClarityValue::none().serialize_to_hex().unwrap()]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(
                r#"{{"okay": true, "result":"0x{}"}}"#,
                ok_settlements_page(&[], Some(&skipped))
                    .serialize_to_hex()
                    .unwrap(),
            ))
            .expect(1)
            .create();

        let pending_then_done = stacks_node_server
            .mock("POST", path.as_str())
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "arguments": [skipped_hex]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(
                r#"{{"okay": true, "result":"0x{}"}}"#,
                ok_settlements_page(&[pending.clone()], None)
                    .serialize_to_hex()
                    .unwrap(),
            ))
            .expect(1)
            .create();

        let client_url = url::Url::parse(stacks_node_server.url().as_str()).unwrap();
        let client = StacksClient::new(client_url).unwrap();

        let registry = RewardClaimRegistry::new(
            QualifiedContractIdentifier::parse(
                "ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039.reward-claim-registry",
            )
            .unwrap(),
            client,
        );

        let result = registry.get_all_pending_settlements().await.unwrap();

        assert_eq!(result, vec![pending]);
        info_mock.assert();
        empty_with_next.assert();
        pending_then_done.assert();
    }
}
