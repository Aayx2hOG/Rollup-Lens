use std::collections::{BTreeMap, BTreeSet};

use chain_core::{
    account_state_value, apply_transaction_to_state, rollup_batch_commitment_from_blocks, Account,
    Block, ChainError, RollupBatchCommitment,
};
use merkle_core::SparseMerkleTree;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BatchValidityWitness {
    pub batch_number: u64,
    pub accounts_before: BTreeMap<String, Account>,
    pub previous_block: Block,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BatchValidityPublicValues {
    pub batch_number: u64,
    pub prev_state_root: String,
    pub new_state_root: String,
    pub transaction_root: String,
    pub block_hash_root: String,
    pub data_hash: String,
    pub transaction_count: usize,
}

pub const EVM_PUBLIC_VALUES_LENGTH: usize = 7 * 32;

impl BatchValidityPublicValues {
    /// Solidity ABI encoding of
    /// `(uint256,bytes32,bytes32,bytes32,bytes32,bytes32,uint256)`.
    pub fn encode_evm(&self) -> Result<[u8; EVM_PUBLIC_VALUES_LENGTH], ValidityError> {
        let mut encoded = [0u8; EVM_PUBLIC_VALUES_LENGTH];
        encoded[24..32].copy_from_slice(&self.batch_number.to_be_bytes());
        for (index, value) in [
            &self.prev_state_root,
            &self.new_state_root,
            &self.transaction_root,
            &self.block_hash_root,
            &self.data_hash,
        ]
        .into_iter()
        .enumerate()
        {
            encoded[(index + 1) * 32..(index + 2) * 32].copy_from_slice(&decode_root(value)?);
        }
        let count = u64::try_from(self.transaction_count).map_err(|_| {
            ValidityError::InvalidPublicValues("transaction count exceeds u64".into())
        })?;
        encoded[6 * 32 + 24..7 * 32].copy_from_slice(&count.to_be_bytes());
        Ok(encoded)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ValidityError {
    #[error("witness state does not match previous block state root")]
    InvalidPreviousState,
    #[error("block {height} state root does not match executed transactions")]
    InvalidExecutedState { height: u64 },
    #[error("invalid batch structure: {0}")]
    InvalidBatch(String),
    #[error("invalid public values: {0}")]
    InvalidPublicValues(String),
}

pub fn verify_batch_witness(
    witness: &BatchValidityWitness,
) -> Result<BatchValidityPublicValues, ValidityError> {
    let entries = witness
        .accounts_before
        .iter()
        .map(|(address, account)| (address.as_bytes(), account_state_value(account)));
    let mut state_tree =
        SparseMerkleTree::new(entries).expect("account addresses must have unique Merkle paths");
    if state_tree.root_hex() != witness.previous_block.state_root {
        return Err(ValidityError::InvalidPreviousState);
    }

    let commitment = rollup_batch_commitment_from_blocks(
        witness.batch_number,
        &witness.previous_block,
        &witness.blocks,
    )
    .map_err(invalid_batch)?;

    let mut accounts = witness.accounts_before.clone();
    for block in &witness.blocks {
        let mut changed_addresses = BTreeSet::new();
        for transaction in &block.transactions {
            apply_transaction_to_state(&mut accounts, transaction).map_err(invalid_batch)?;
            changed_addresses.insert(transaction.from.as_str());
            changed_addresses.insert(transaction.to.as_str());
        }
        for address in changed_addresses {
            let account = accounts
                .get(address)
                .expect("applied transaction accounts must exist");
            state_tree.update(address.as_bytes(), Some(&account_state_value(account)));
        }
        if state_tree.root_hex() != block.state_root {
            return Err(ValidityError::InvalidExecutedState {
                height: block.height,
            });
        }
    }

    Ok(public_values(commitment))
}

fn public_values(commitment: RollupBatchCommitment) -> BatchValidityPublicValues {
    BatchValidityPublicValues {
        batch_number: commitment.batch_number,
        prev_state_root: commitment.prev_state_root,
        new_state_root: commitment.new_state_root,
        transaction_root: commitment.transaction_root,
        block_hash_root: commitment.block_hash_root,
        data_hash: commitment.data_hash,
        transaction_count: commitment.transaction_count,
    }
}

fn invalid_batch(error: ChainError) -> ValidityError {
    ValidityError::InvalidBatch(error.to_string())
}

fn decode_root(value: &str) -> Result<[u8; 32], ValidityError> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    if value.len() != 64 {
        return Err(ValidityError::InvalidPublicValues(
            "roots must contain exactly 32 bytes".into(),
        ));
    }
    let mut result = [0u8; 32];
    for (index, byte) in result.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| ValidityError::InvalidPublicValues("root is not hexadecimal".into()))?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use chain_core::{
        add_pending_transaction, finalize_genesis, new_chain, produce_block, sign_transfer, Account,
    };
    use wallet_core::{address_from_public_key, create_keypair};

    use super::*;

    #[test]
    fn verifies_the_same_transition_committed_by_the_chain() {
        let alice = create_keypair();
        let bob = create_keypair();
        let alice_address = address_from_public_key(&alice.public_key_base64).unwrap();
        let bob_address = address_from_public_key(&bob.public_key_base64).unwrap();
        let genesis_accounts = vec![Account {
            address: alice_address,
            balance: 100,
            nonce: 0,
        }];
        let accounts_before = genesis_accounts
            .iter()
            .map(|account| (account.address.clone(), account.clone()))
            .collect();
        let mut chain = new_chain(genesis_accounts).unwrap();
        let previous_block = finalize_genesis(&mut chain).unwrap();
        let transfer = sign_transfer(&alice.secret_key_base64, bob_address, 40, 0).unwrap();
        add_pending_transaction(&mut chain, transfer).unwrap();
        let block = produce_block(&mut chain).unwrap();

        let values = verify_batch_witness(&BatchValidityWitness {
            batch_number: 0,
            accounts_before,
            previous_block: previous_block.clone(),
            blocks: vec![block.clone()],
        })
        .unwrap();

        assert_eq!(values.prev_state_root, previous_block.state_root);
        assert_eq!(values.new_state_root, block.state_root);
        assert_eq!(values.transaction_count, 1);
    }

    #[test]
    fn rejects_a_forged_post_state_root() {
        let alice = create_keypair();
        let alice_address = address_from_public_key(&alice.public_key_base64).unwrap();
        let genesis_accounts = vec![Account {
            address: alice_address,
            balance: 100,
            nonce: 0,
        }];
        let accounts_before = genesis_accounts
            .iter()
            .map(|account| (account.address.clone(), account.clone()))
            .collect();
        let mut chain = new_chain(genesis_accounts).unwrap();
        let previous_block = finalize_genesis(&mut chain).unwrap();
        let mut forged_block = previous_block.clone();
        forged_block.height = 1;
        forged_block.parent_hash = previous_block.block_hash.clone();
        forged_block.state_root = "ff".repeat(32);

        assert!(verify_batch_witness(&BatchValidityWitness {
            batch_number: 0,
            accounts_before,
            previous_block,
            blocks: vec![forged_block],
        })
        .is_err());
    }

    #[test]
    fn evm_public_values_match_solidity_abi_layout() {
        let values = BatchValidityPublicValues {
            batch_number: 7,
            prev_state_root: "11".repeat(32),
            new_state_root: "22".repeat(32),
            transaction_root: "33".repeat(32),
            block_hash_root: "44".repeat(32),
            data_hash: "55".repeat(32),
            transaction_count: 9,
        };

        let encoded = values.encode_evm().unwrap();
        assert_eq!(encoded.len(), 224);
        assert_eq!(&encoded[24..32], &7u64.to_be_bytes());
        assert_eq!(&encoded[32..64], &[0x11; 32]);
        assert_eq!(&encoded[192 + 24..224], &9u64.to_be_bytes());
    }
}
