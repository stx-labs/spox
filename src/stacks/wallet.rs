//! Single-sig Stacks wallet for building and signing transactions.

use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use blockstack_lib::chainstate::stacks::SinglesigHashMode;
use blockstack_lib::chainstate::stacks::SinglesigSpendingCondition;
use blockstack_lib::chainstate::stacks::StacksTransaction;
use blockstack_lib::chainstate::stacks::TransactionAnchorMode;
use blockstack_lib::chainstate::stacks::TransactionAuth;
use blockstack_lib::chainstate::stacks::TransactionAuthFlags;
use blockstack_lib::chainstate::stacks::TransactionPayload;
use blockstack_lib::chainstate::stacks::TransactionPostConditionMode;
use blockstack_lib::chainstate::stacks::TransactionPublicKeyEncoding;
use blockstack_lib::chainstate::stacks::TransactionSpendingCondition;
use blockstack_lib::chainstate::stacks::TransactionVersion;
use blockstack_lib::core::CHAIN_ID_MAINNET;
use blockstack_lib::util::secp256k1::MessageSignature;
use blockstack_lib::util::secp256k1::Secp256k1PublicKey;
use clarity::types::chainstate::StacksAddress;
use secp256k1::PublicKey;
use secp256k1::SECP256K1;
use secp256k1::SecretKey;
use secp256k1::ecdsa::RecoverableSignature;

/// A single-sig Stacks wallet that tracks a local nonce counter.
///
/// The nonce starts at whatever value is passed to [`StacksWallet::new`]
/// and must be manually incremented or set.
///
/// The `chain_id` should be the exact value from the stacks node's
/// `/v2/info` `network_id` field.
#[derive(Debug)]
pub struct StacksWallet {
    /// Secp256k1 private key used to sign transactions.
    secret_key: SecretKey,
    /// Stacks address derived from [`Self::secret_key`] for this chain.
    address: StacksAddress,
    /// Stacks chain id written into signed transactions.
    chain_id: u32,
    /// Local next-nonce counter.
    nonce: AtomicU64,
}

impl StacksWallet {
    /// Create a new single-sig wallet for the given chain id.
    ///
    /// The Stacks address is the P2PKH address derived from the compressed public key.
    /// `chain_id` should be the `network_id` returned by `GET /v2/info`.
    pub fn new(secret_key: SecretKey, chain_id: u32, nonce: u64) -> Self {
        let public_key = PublicKey::from_secret_key(SECP256K1, &secret_key);
        let stacks_public_key = Secp256k1PublicKey::from_slice(&public_key.serialize())
            .expect("compressed secp256k1 public key should convert to stacks public key");

        let is_mainnet = chain_id == CHAIN_ID_MAINNET;
        let address = StacksAddress::p2pkh(is_mainnet, &stacks_public_key);

        Self {
            secret_key,
            address,
            chain_id,
            nonce: AtomicU64::new(nonce),
        }
    }

    /// The Stacks P2PKH address for this wallet.
    pub fn address(&self) -> &StacksAddress {
        &self.address
    }

    /// Whether this wallet is configured for Stacks mainnet.
    pub fn is_mainnet(&self) -> bool {
        self.chain_id == CHAIN_ID_MAINNET
    }

    /// Set the next nonce to the provided value.
    pub fn set_nonce(&self, value: u64) {
        self.nonce.store(value, Ordering::Relaxed);
    }

    /// Increment the wallet nonce by 1.
    pub fn increment_nonce(&self) {
        self.nonce.fetch_add(1, Ordering::Relaxed);
    }

    /// Build an unsigned single-sig spending condition.
    ///
    /// The returned spending condition has a fee and nonce set, but an
    /// empty signature.
    pub fn as_unsigned_tx_auth(&self, tx_fee: u64) -> SinglesigSpendingCondition {
        SinglesigSpendingCondition {
            signer: self.address.bytes().clone(),
            nonce: self.nonce.load(Ordering::Relaxed),
            tx_fee,
            hash_mode: SinglesigHashMode::P2PKH,
            key_encoding: TransactionPublicKeyEncoding::Compressed,
            signature: MessageSignature::empty(),
        }
    }

    /// Build and sign a Stacks transaction for the given payload.
    pub fn sign_tx(&self, payload: TransactionPayload, tx_fee: u64) -> StacksTransaction {
        use TransactionSpendingCondition::Singlesig;

        let version = if self.is_mainnet() {
            TransactionVersion::Mainnet
        } else {
            TransactionVersion::Testnet
        };

        let auth = self.as_unsigned_tx_auth(tx_fee);
        let mut tx = StacksTransaction {
            version,
            chain_id: self.chain_id,
            auth: TransactionAuth::Standard(Singlesig(auth)),
            anchor_mode: TransactionAnchorMode::Any,
            // Uses originator post-condition mode to deny asset movement for our
            // assets allow it for all other principals.
            post_condition_mode: TransactionPostConditionMode::Originator,
            post_conditions: Vec::new(),
            payload,
        };

        let msg = secp256k1::Message::from_digest(tx_digest(&tx));
        let signature = SECP256K1.sign_ecdsa_recoverable(&msg, &self.secret_key);

        let TransactionAuth::Standard(Singlesig(cond)) = &mut tx.auth else {
            unreachable!("what!? we just created this a few lines above");
        };
        cond.set_signature(signature.as_stacks_sig());

        tx
    }
}

/// Construct the digest that each signer needs to sign from a given
/// transaction.
///
/// # Note
///
/// This function follows the same procedure as the
/// [`TransactionSpendingCondition::next_signature`] function in
/// stacks-core, except that it stops after the digest is created.
fn tx_digest(tx: &StacksTransaction) -> [u8; 32] {
    let mut cleared_tx = tx.clone();
    cleared_tx.auth = cleared_tx.auth.into_initial_sighash_auth();

    let sighash = cleared_tx.txid();
    let flags = TransactionAuthFlags::AuthStandard;
    let tx_fee = tx.get_tx_fee();
    let nonce = tx.get_origin_nonce();

    TransactionSpendingCondition::make_sighash_presign(&sighash, &flags, tx_fee, nonce).into_bytes()
}

/// This trait is to add additional functionality to the
/// [`RecoverableSignature`] type
pub trait RecoverableEcdsaSignature: Sized {
    /// Convert a recoverable signature into compact bytes.
    fn to_byte_array(&self) -> [u8; 65];
    /// Convert this type to the equivalent stacks signature
    fn as_stacks_sig(&self) -> MessageSignature {
        MessageSignature(self.to_byte_array())
    }
}

impl RecoverableEcdsaSignature for RecoverableSignature {
    /// Convert a recoverable signature into a byte array.
    ///
    /// The [`RecoverableSignature`] type is a wrapper of a wrapper for
    /// [u8; 65]. Unfortunately, the outermost wrapper type does not
    /// provide a way to get at the underlying bytes except through the
    /// [`RecoverableSignature::serialize_compact`] function, so we use
    /// that function to extract the bytes.
    ///
    /// This function is basically lifted from stacks-core at:
    /// https://github.com/stacks-network/stacks-core/blob/35d0840c626d258f1e2d72becdcf207a0572ddcd/stacks-common/src/util/secp256k1.rs#L88-L95
    fn to_byte_array(&self) -> [u8; 65] {
        let (recovery_id, bytes) = self.serialize_compact();
        let mut ret_bytes = [0u8; 65];
        // The recovery ID will be 0, 1, 2, or 3 as described in the secp256k1 docs:
        // https://docs.rs/secp256k1/0.30.0/secp256k1/ecdsa/enum.RecoveryId.html
        ret_bytes[0] = recovery_id.to_i32() as u8;

        ret_bytes[1..].copy_from_slice(&bytes[..]);
        ret_bytes
    }
}

#[cfg(test)]
mod tests {
    use blockstack_lib::chainstate::stacks::TokenTransferMemo;
    use blockstack_lib::chainstate::stacks::TransactionAuthVerificationMode;
    use blockstack_lib::chainstate::stacks::TransactionPayload;
    use blockstack_lib::core::CHAIN_ID_TESTNET;
    use clarity::types::chainstate::StacksAddress;
    use clarity::vm::types::PrincipalData;
    use secp256k1::SecretKey;

    use super::*;

    fn test_secret_key() -> SecretKey {
        SecretKey::from_slice(&[0x11; 32]).expect("valid secret key")
    }

    fn token_transfer_payload(recipient: &StacksAddress) -> TransactionPayload {
        let principal = PrincipalData::from(recipient.clone());
        TransactionPayload::TokenTransfer(principal, 1, TokenTransferMemo([0u8; 34]))
    }

    impl StacksWallet {
        fn nonce(&self) -> u64 {
            self.nonce.load(Ordering::SeqCst)
        }
    }

    #[test]
    fn as_unsigned_tx_auth_uses_current_nonce_without_incrementing() {
        let wallet = StacksWallet::new(test_secret_key(), CHAIN_ID_TESTNET, 7);

        let auth0 = wallet.as_unsigned_tx_auth(1000);
        assert_eq!(auth0.nonce, 7);
        assert_eq!(wallet.nonce(), 7);

        let auth1 = wallet.as_unsigned_tx_auth(1000);
        assert_eq!(auth1.nonce, 7);
        assert_eq!(wallet.nonce(), 7);
    }

    #[test]
    fn sign_tx_produces_verifiable_transaction() {
        use TransactionAuthVerificationMode::EnforceLowS;
        let wallet = StacksWallet::new(test_secret_key(), CHAIN_ID_TESTNET, 0);
        let payload = token_transfer_payload(wallet.address());

        let tx = wallet.sign_tx(payload, 1000);
        assert!(tx.verify(EnforceLowS).is_ok());
        assert_eq!(tx.chain_id, CHAIN_ID_TESTNET);
        assert_eq!(
            tx.post_condition_mode,
            TransactionPostConditionMode::Originator
        );
        assert!(tx.post_conditions.is_empty());
        assert_eq!(wallet.nonce(), 0);
    }

    #[test]
    fn set_nonce_updates_counter() {
        let wallet = StacksWallet::new(test_secret_key(), CHAIN_ID_TESTNET, 0);
        wallet.set_nonce(42);
        assert_eq!(wallet.nonce(), 42);

        let auth = wallet.as_unsigned_tx_auth(1);
        assert_eq!(auth.nonce, 42);
        assert_eq!(wallet.nonce(), 42);
    }

    #[test]
    fn increment_nonce_advances_counter() {
        let wallet = StacksWallet::new(test_secret_key(), CHAIN_ID_TESTNET, 7);
        wallet.increment_nonce();
        assert_eq!(wallet.nonce(), 8);

        let auth = wallet.as_unsigned_tx_auth(1000);
        assert_eq!(auth.nonce, 8);
        assert_eq!(wallet.nonce(), 8);
    }

    #[test]
    fn mainnet_chain_id_selects_mainnet_address_version() {
        let mainnet = StacksWallet::new(test_secret_key(), CHAIN_ID_MAINNET, 0);
        let testnet = StacksWallet::new(test_secret_key(), CHAIN_ID_TESTNET, 0);

        assert!(mainnet.is_mainnet());
        assert!(!testnet.is_mainnet());
        assert_ne!(mainnet.address().to_string(), testnet.address().to_string());
        assert!(mainnet.address().to_string().starts_with('S'));
        assert!(testnet.address().to_string().starts_with('S'));
        // Mainnet singlesig addresses use the SP prefix; testnet uses ST.
        assert!(mainnet.address().to_string().starts_with("SP"));
        assert!(testnet.address().to_string().starts_with("ST"));
    }
}
