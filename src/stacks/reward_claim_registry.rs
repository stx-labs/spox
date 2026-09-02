//! Client for the on-chain reward claim registry.

use clarity::types::chainstate::StacksAddress;
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

/// Arguments for one `process-reward-claims` contract call.
///
/// All [`Self::stakers`] share [`Self::signer_manager`], and the list
/// length is at most [`MAX_STAKERS_LENGTH`]. (TODO: enforce this at
/// construction time)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessRewardClaimsBatch {
    /// Signer-manager trait principal passed to `process-reward-claims`.
    pub signer_manager: QualifiedContractIdentifier,
    /// Staker principals to claim for in this call (1..=100).
    pub stakers: Vec<PrincipalData>,
    /// The address that deployed the rewards claim registry.
    pub deployer: StacksAddress,
}

/// Client for querying the on-chain reward claim registry contract.
#[derive(Debug, Clone)]
pub struct RewardClaimRegistry {
    /// The deployer of the registry smart contract.
    contract_principal: StacksAddress,
    /// The name of the registry smart contract.
    contract_name: ContractName,
    /// The client used to make the requests.
    client: StacksClient,
}

impl RewardClaimRegistry {
    /// Create a new reward claim registry client.
    pub fn new(contract: QualifiedContractIdentifier, client: StacksClient) -> Self {
        let contract_principal = contract.issuer.into();

        Self {
            contract_name: contract.name,
            contract_principal,
            client,
        }
    }

    /// Fetch a page of pending claims from the registry.
    ///
    /// Pass `None` for `cursor` to start at the head of the registration
    /// linked list. To paginate, pass [`PendingClaimsPage::next`] from the
    /// previous page. `next == None` means the walk reached the tail; an
    /// empty `claims` list alone does not.
    pub async fn get_pending_claims(
        &self,
        cursor: Option<&RegistrationKey>,
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
                &self.contract_principal,
                &self.contract_name,
                &ClarityName::from("get-pending-claims"),
                &self.contract_principal,
                &[cursor_arg],
            )
            .await?;

        let ClarityValue::Response(response) = result else {
            return Err(Error::InvalidStacksResponse("expected a response"));
        };

        PendingClaimsPage::try_from(*response.data)
    }

    /// Fetch every pending claim by paging through `get-pending-claims`.
    ///
    /// Continues while the page's `next` cursor is `Some`, including pages
    /// that return no rows (ticks spent on non-pending registrations).
    pub async fn get_all_pending_claims(&self) -> Result<Vec<PendingClaim>, Error> {
        let mut all = Vec::new();
        let mut cursor: Option<RegistrationKey> = None;

        loop {
            let page = self.get_pending_claims(cursor.as_ref()).await?;
            all.extend(page.claims);
            match page.next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        Ok(all)
    }
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

#[cfg(test)]
mod tests {
    use bitcoincore_rpc::jsonrpc::serde_json;
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

        let result = registry.get_pending_claims(None).await.unwrap();

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

        let result = registry.get_pending_claims(Some(&cursor)).await.unwrap();

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

        let result = registry.get_pending_claims(None).await.unwrap();

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

        let mut stacks_node_server = mockito::Server::new_async().await;
        let path = "/v2/contracts/call-read/ST2SBXRBJJTH7GV5J93HJ62W2NRRQ46XYBK92Y039/reward-claim-registry/get-pending-claims?tip=latest";

        let empty_with_next = stacks_node_server
            .mock("POST", path)
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
            .mock("POST", path)
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
        empty_with_next.assert();
        pending_then_done.assert();
    }
}
