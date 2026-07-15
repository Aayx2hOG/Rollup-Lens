use std::collections::{BTreeMap, BTreeSet};

use chain_core::{
    account_state_value, apply_deposit_to_state, apply_transaction_to_state,
    apply_withdrawal_to_state, rollup_batch_commitment_from_blocks, validate_deposit_sequence,
    Account, Block, ChainError, RollupBatchCommitment,
};
use merkle_core::SparseMerkleTree;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BatchValidityWitness {
    pub batch_number: u64,
    #[serde(default = "default_l1_deposit_inbox")]
    pub l1_deposit_inbox: String,
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
    #[serde(default)]
    pub deposit_count: usize,
    #[serde(default)]
    pub deposit_root: String,
    #[serde(default)]
    pub first_deposit_id: Option<u64>,
    #[serde(default)]
    pub last_deposit_id: Option<u64>,
    #[serde(default)]
    pub withdrawal_count: usize,
    #[serde(default)]
    pub withdrawal_root: String,
    #[serde(default = "default_l1_deposit_inbox")]
    pub l1_deposit_inbox: String,
}

pub const EVM_PUBLIC_VALUES_LENGTH: usize = 14 * 32;

impl BatchValidityPublicValues {
    /// Solidity ABI encoding of
    /// `(uint256,bytes32,bytes32,bytes32,bytes32,bytes32,bytes32,uint256,uint256,uint256,uint256,bytes32,uint256,address)`.
    pub fn encode_evm(&self) -> Result<[u8; EVM_PUBLIC_VALUES_LENGTH], ValidityError> {
        let mut encoded = [0u8; EVM_PUBLIC_VALUES_LENGTH];
        encoded[24..32].copy_from_slice(&self.batch_number.to_be_bytes());
        for (index, value) in [
            &self.prev_state_root,
            &self.new_state_root,
            &self.transaction_root,
            &self.block_hash_root,
            &self.data_hash,
            &self.deposit_root,
        ]
        .into_iter()
        .enumerate()
        {
            encoded[(index + 1) * 32..(index + 2) * 32].copy_from_slice(&decode_root(value)?);
        }
        let first_deposit_id = self.first_deposit_id.unwrap_or(0);
        let last_deposit_id = self.last_deposit_id.unwrap_or(0);
        encoded[7 * 32 + 24..8 * 32].copy_from_slice(&first_deposit_id.to_be_bytes());
        encoded[8 * 32 + 24..9 * 32].copy_from_slice(&last_deposit_id.to_be_bytes());
        let deposit_count = u64::try_from(self.deposit_count)
            .map_err(|_| ValidityError::InvalidPublicValues("deposit count exceeds u64".into()))?;
        encoded[9 * 32 + 24..10 * 32].copy_from_slice(&deposit_count.to_be_bytes());
        let count = u64::try_from(self.transaction_count).map_err(|_| {
            ValidityError::InvalidPublicValues("transaction count exceeds u64".into())
        })?;
        encoded[10 * 32 + 24..11 * 32].copy_from_slice(&count.to_be_bytes());
        encoded[11 * 32..12 * 32].copy_from_slice(&decode_root(&self.withdrawal_root)?);
        let withdrawal_count = u64::try_from(self.withdrawal_count).map_err(|_| {
            ValidityError::InvalidPublicValues("withdrawal count exceeds u64".into())
        })?;
        encoded[12 * 32 + 24..13 * 32].copy_from_slice(&withdrawal_count.to_be_bytes());
        encoded[13 * 32..14 * 32].copy_from_slice(&decode_address_word(&self.l1_deposit_inbox)?);
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
        validate_deposit_sequence(&block.deposits).map_err(invalid_batch)?;
        for deposit in &block.deposits {
            if !deposit
                .inbox_address
                .eq_ignore_ascii_case(&witness.l1_deposit_inbox)
            {
                return Err(ValidityError::InvalidBatch(format!(
                    "deposit {} came from unexpected inbox",
                    deposit.deposit_id
                )));
            }
            apply_deposit_to_state(&mut accounts, deposit).map_err(invalid_batch)?;
            changed_addresses.insert(deposit.recipient_l2_address.as_str());
        }
        for transaction in &block.transactions {
            apply_transaction_to_state(&mut accounts, transaction).map_err(invalid_batch)?;
            changed_addresses.insert(transaction.from.as_str());
            changed_addresses.insert(transaction.to.as_str());
        }
        for withdrawal in &block.withdrawals {
            apply_withdrawal_to_state(&mut accounts, withdrawal).map_err(invalid_batch)?;
            changed_addresses.insert(withdrawal.from.as_str());
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

    Ok(public_values(commitment, witness.l1_deposit_inbox.clone()))
}

fn public_values(
    commitment: RollupBatchCommitment,
    l1_deposit_inbox: String,
) -> BatchValidityPublicValues {
    BatchValidityPublicValues {
        batch_number: commitment.batch_number,
        prev_state_root: commitment.prev_state_root,
        new_state_root: commitment.new_state_root,
        transaction_root: commitment.transaction_root,
        block_hash_root: commitment.block_hash_root,
        data_hash: commitment.data_hash,
        transaction_count: commitment.transaction_count,
        deposit_count: commitment.deposit_count,
        deposit_root: commitment.deposit_root,
        first_deposit_id: commitment.first_deposit_id,
        last_deposit_id: commitment.last_deposit_id,
        withdrawal_count: commitment.withdrawal_count,
        withdrawal_root: commitment.withdrawal_root,
        l1_deposit_inbox,
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

fn decode_address_word(value: &str) -> Result<[u8; 32], ValidityError> {
    let value = value
        .strip_prefix("0x")
        .ok_or_else(|| ValidityError::InvalidPublicValues("address must use 0x prefix".into()))?;
    if value.len() != 40 {
        return Err(ValidityError::InvalidPublicValues(
            "address must contain exactly 20 bytes".into(),
        ));
    }
    let mut result = [0u8; 32];
    for index in 0..20 {
        result[12 + index] = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| ValidityError::InvalidPublicValues("address is not hexadecimal".into()))?;
    }
    Ok(result)
}

fn default_l1_deposit_inbox() -> String {
    "0x0000000000000000000000000000000000000000".to_string()
}

#[cfg(test)]
mod tests {
    use chain_core::{
        finalize_genesis, new_chain, produce_block_from_indexed_state_with_deposits, sign_transfer,
        Account, L1Deposit,
    };
    use wallet_core::create_keypair;

    use super::*;
    const L1_DEPOSIT_INBOX: &str = "0x2222222222222222222222222222222222222222";

    #[test]
    fn verifies_the_same_transition_committed_by_the_chain() {
        let alice = create_keypair();
        let bob = create_keypair();
        let alice_address = alice.address.clone();
        let bob_address = bob.address.clone();
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
        let transfer = sign_transfer(&alice.secret_key_hex, bob_address, 40, 0).unwrap();
        let mut accounts = chain.accounts.clone();
        let block = produce_block_from_indexed_state_with_deposits(
            &mut accounts,
            &previous_block,
            Vec::new(),
            vec![transfer],
        )
        .unwrap();

        let values = verify_batch_witness(&BatchValidityWitness {
            batch_number: 0,
            l1_deposit_inbox: L1_DEPOSIT_INBOX.to_string(),
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
    fn verifies_deposit_credits_inside_the_witness() {
        let alice = create_keypair();
        let alice_address = alice.address.clone();
        let accounts_before = BTreeMap::new();
        let mut chain = new_chain(Vec::new()).unwrap();
        let previous_block = finalize_genesis(&mut chain).unwrap();
        let deposit = L1Deposit {
            chain_id: 31337,
            inbox_address: "0x2222222222222222222222222222222222222222".to_owned(),
            deposit_id: 0,
            depositor_l1_address: "0x3333333333333333333333333333333333333333".to_owned(),
            recipient_l2_address: alice_address,
            amount: 25,
            rolling_commitment: format!("0x{}", "4".repeat(64)),
        };
        let mut accounts = BTreeMap::new();
        let block = produce_block_from_indexed_state_with_deposits(
            &mut accounts,
            &previous_block,
            vec![deposit],
            Vec::new(),
        )
        .unwrap();

        let values = verify_batch_witness(&BatchValidityWitness {
            batch_number: 0,
            l1_deposit_inbox: L1_DEPOSIT_INBOX.to_string(),
            accounts_before,
            previous_block: previous_block.clone(),
            blocks: vec![block.clone()],
        })
        .unwrap();

        assert_eq!(values.new_state_root, block.state_root);
        assert_eq!(values.deposit_count, 1);
        assert_eq!(values.first_deposit_id, Some(0));
        assert_eq!(values.last_deposit_id, Some(0));
    }

    #[test]
    fn rejects_a_forged_post_state_root() {
        let alice = create_keypair();
        let alice_address = alice.address.clone();
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
            l1_deposit_inbox: L1_DEPOSIT_INBOX.to_string(),
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
            deposit_count: 0,
            deposit_root: "00".repeat(32),
            first_deposit_id: None,
            last_deposit_id: None,
            withdrawal_count: 0,
            withdrawal_root: "66".repeat(32),
            l1_deposit_inbox: L1_DEPOSIT_INBOX.to_string(),
        };

        let encoded = values.encode_evm().unwrap();
        assert_eq!(encoded.len(), 448);
        assert_eq!(&encoded[24..32], &7u64.to_be_bytes());
        assert_eq!(&encoded[32..64], &[0x11; 32]);
        assert_eq!(&encoded[10 * 32 + 24..11 * 32], &9u64.to_be_bytes());
        assert_eq!(&encoded[11 * 32..12 * 32], &[0x66; 32]);
        assert_eq!(&encoded[13 * 32 + 12..14 * 32], &[0x22; 20]);
    }
}
