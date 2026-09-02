//! Contains functionality for interacting with the Bitcoin blockchain

use bitcoin;

pub mod chain_tip;
pub mod node;
pub mod wallet;

/// Bitcoin chain tip
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct BlockRef {
    /// The height of the block in the bitcoin blockchain.
    pub block_height: u64,
    /// Bitcoin block hash. It uniquely identifies the bitcoin block.
    pub block_hash: bitcoin::BlockHash,
}

impl std::fmt::Display for BlockRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Block(hash={}, height={})",
            self.block_hash, self.block_height
        )
    }
}

/// Unspent transaction output
#[derive(Debug)]
pub struct Utxo {
    /// Transaction id
    pub txid: bitcoin::Txid,
    /// Output index
    pub vout: u32,
    /// The script pubkey of this output
    pub script_pub_key: bitcoin::ScriptBuf,
    /// Amount of this output
    pub amount: bitcoin::Amount,
    /// Block height of this transaction
    pub block_height: u64,
}
