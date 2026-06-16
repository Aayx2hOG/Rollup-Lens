use std::collections::BTreeMap;

use merkle_core::{encode_hex, root_hex};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use wallet_core::{
    address_from_public_key, public_key_from_secret, sign_message, verify_message, WalletError,
};

const NULL_PARENT_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Account {
    pub address: String,
    pub balance: u64,
    pub nonce: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Transfer {
    pub from: String,
    pub to: String,
    pub amount: u64,
    pub nonce: u64,
    pub public_key_base64: String,
    pub signature_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Block {
    pub height: u64,
    pub parent_hash: String,
    pub transactions: Vec<Transfer>,
    pub state_root: String,
    pub block_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Chain {
    pub genesis_accounts: Vec<Account>,
    pub accounts: BTreeMap<String, Account>,
    pub pending_transactions: Vec<Transfer>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct UnsignedTransfer<'a> {
    from: &'a str,
    to: &'a str,
    amount: u64,
    nonce: u64,
    public_key_base64: &'a str,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct BlockCommitment<'a> {
    height: u64,
    parent_hash: &'a str,
    transactions: &'a [Transfer],
    state_root: &'a str,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ChainError {
    #[error("duplicate genesis account {0}")]
    DuplicateGenesisAccount(String),
    #[error("account {0} already exists")]
    AccountAlreadyExists(String),
    #[error("cannot add genesis accounts after genesis has been finalized")]
    GenesisLocked,
    #[error("genesis block has already been finalized")]
    GenesisAlreadyFinalized,
    #[error("genesis block must be finalized before producing transaction blocks")]
    GenesisNotFinalized,
    #[error("genesis block cannot contain transactions")]
    InvalidGenesisTransactions,
    #[error("transfer amount must be greater than zero")]
    ZeroAmount,
    #[error("sender account {0} does not exist")]
    MissingSender(String),
    #[error("sender address does not match public key")]
    SenderPublicKeyMismatch,
    #[error("invalid transaction signature")]
    InvalidSignature,
    #[error("invalid nonce for {address}: expected {expected}, got {actual}")]
    InvalidNonce {
        address: String,
        expected: u64,
        actual: u64,
    },
    #[error("insufficient funds for {address}: balance {balance}, amount {amount}")]
    InsufficientFunds {
        address: String,
        balance: u64,
        amount: u64,
    },
    #[error("balance overflow for account {0}")]
    BalanceOverflow(String),
    #[error("wallet error: {0}")]
    Wallet(String),
    #[error("block height mismatch: expected {expected}, got {actual}")]
    InvalidHeight { expected: u64, actual: u64 },
    #[error("block {height} has invalid parent hash")]
    InvalidParentHash { height: u64 },
    #[error("block {height} has invalid state root")]
    InvalidStateRoot { height: u64 },
    #[error("block {height} has invalid block hash")]
    InvalidBlockHash { height: u64 },
    #[error("current account snapshot does not match replayed chain state")]
    AccountSnapshotMismatch,
}

impl From<WalletError> for ChainError {
    fn from(error: WalletError) -> Self {
        Self::Wallet(error.to_string())
    }
}

pub fn new_chain(genesis_accounts: Vec<Account>) -> Result<Chain, ChainError> {
    let accounts = accounts_from_genesis(&genesis_accounts)?;

    Ok(Chain {
        genesis_accounts,
        accounts,
        pending_transactions: Vec::new(),
        blocks: Vec::new(),
    })
}

pub fn add_genesis_account(
    chain: &mut Chain,
    address: String,
    balance: u64,
) -> Result<(), ChainError> {
    if !chain.blocks.is_empty() {
        return Err(ChainError::GenesisLocked);
    }
    if chain.accounts.contains_key(&address) {
        return Err(ChainError::AccountAlreadyExists(address));
    }

    let account = Account {
        address: address.clone(),
        balance,
        nonce: 0,
    };
    chain.genesis_accounts.push(account.clone());
    chain.accounts.insert(address, account);
    Ok(())
}

pub fn finalize_genesis(chain: &mut Chain) -> Result<Block, ChainError> {
    if !chain.blocks.is_empty() {
        return Err(ChainError::GenesisAlreadyFinalized);
    }

    let state_root = state_root(&chain.accounts);
    let transactions = Vec::new();
    let block_hash = block_hash(0, NULL_PARENT_HASH, &transactions, &state_root);
    let block = Block {
        height: 0,
        parent_hash: NULL_PARENT_HASH.to_owned(),
        transactions,
        state_root,
        block_hash,
    };

    chain.blocks.push(block.clone());
    Ok(block)
}

pub fn sign_transfer(
    secret_key_base64: &str,
    to: String,
    amount: u64,
    nonce: u64,
) -> Result<Transfer, ChainError> {
    let public_key_base64 = public_key_from_secret(secret_key_base64)?;
    let from = address_from_public_key(&public_key_base64)?;
    let payload = transfer_payload(&from, &to, amount, nonce, &public_key_base64);
    let signed = sign_message(secret_key_base64, &payload)?;

    Ok(Transfer {
        from,
        to,
        amount,
        nonce,
        public_key_base64,
        signature_base64: signed.signature_base64,
    })
}

pub fn add_pending_transaction(chain: &mut Chain, transaction: Transfer) -> Result<(), ChainError> {
    ensure_genesis_finalized(chain)?;
    let pending_state = pending_state(chain)?;
    validate_transaction(&pending_state, &transaction)?;
    chain.pending_transactions.push(transaction);
    Ok(())
}

pub fn next_nonce(chain: &Chain, address: &str) -> Result<u64, ChainError> {
    ensure_genesis_finalized(chain)?;
    let accounts = pending_state(chain)?;
    accounts
        .get(address)
        .map(|account| account.nonce)
        .ok_or_else(|| ChainError::MissingSender(address.to_owned()))
}

pub fn produce_block(chain: &mut Chain) -> Result<Block, ChainError> {
    ensure_genesis_finalized(chain)?;
    let mut next_accounts = chain.accounts.clone();
    for transaction in &chain.pending_transactions {
        apply_transaction(&mut next_accounts, transaction)?;
    }

    let state_root = state_root(&next_accounts);
    let height = chain.blocks.len() as u64;
    let parent_hash = chain
        .blocks
        .last()
        .map(|block| block.block_hash.clone())
        .expect("genesis finalization ensures at least one block")
        .clone();
    let transactions = std::mem::take(&mut chain.pending_transactions);
    let block_hash = block_hash(height, &parent_hash, &transactions, &state_root);

    let block = Block {
        height,
        parent_hash,
        transactions,
        state_root,
        block_hash,
    };

    chain.accounts = next_accounts;
    chain.blocks.push(block.clone());
    Ok(block)
}

pub fn validate_chain(chain: &Chain) -> Result<(), ChainError> {
    let mut accounts = accounts_from_genesis(&chain.genesis_accounts)?;
    let mut expected_parent = NULL_PARENT_HASH.to_owned();

    for (expected_height, block) in chain.blocks.iter().enumerate() {
        let expected_height = expected_height as u64;
        if block.height != expected_height {
            return Err(ChainError::InvalidHeight {
                expected: expected_height,
                actual: block.height,
            });
        }

        if block.parent_hash != expected_parent {
            return Err(ChainError::InvalidParentHash {
                height: block.height,
            });
        }

        if block.height == 0 {
            if !block.transactions.is_empty() {
                return Err(ChainError::InvalidGenesisTransactions);
            }
        } else {
            for transaction in &block.transactions {
                apply_transaction(&mut accounts, transaction)?;
            }
        }

        let expected_state_root = state_root(&accounts);
        if block.state_root != expected_state_root {
            return Err(ChainError::InvalidStateRoot {
                height: block.height,
            });
        }

        let expected_block_hash = block_hash(
            block.height,
            &block.parent_hash,
            &block.transactions,
            &block.state_root,
        );
        if block.block_hash != expected_block_hash {
            return Err(ChainError::InvalidBlockHash {
                height: block.height,
            });
        }

        expected_parent = block.block_hash.clone();
    }

    if accounts != chain.accounts {
        return Err(ChainError::AccountSnapshotMismatch);
    }

    Ok(())
}

pub fn state_root(accounts: &BTreeMap<String, Account>) -> String {
    let leaves = accounts
        .values()
        .map(|account| {
            json!({
                "address": account.address,
                "balance": account.balance,
                "nonce": account.nonce,
            })
        })
        .collect::<Vec<_>>();

    root_hex(&leaves)
}

pub fn validate_transaction(
    accounts: &BTreeMap<String, Account>,
    transaction: &Transfer,
) -> Result<(), ChainError> {
    if transaction.amount == 0 {
        return Err(ChainError::ZeroAmount);
    }

    let derived_address = address_from_public_key(&transaction.public_key_base64)?;
    if derived_address != transaction.from {
        return Err(ChainError::SenderPublicKeyMismatch);
    }

    let payload = transfer_payload(
        &transaction.from,
        &transaction.to,
        transaction.amount,
        transaction.nonce,
        &transaction.public_key_base64,
    );
    if !verify_message(
        &transaction.public_key_base64,
        &payload,
        &transaction.signature_base64,
    )? {
        return Err(ChainError::InvalidSignature);
    }

    let sender = accounts
        .get(&transaction.from)
        .ok_or_else(|| ChainError::MissingSender(transaction.from.clone()))?;
    if sender.nonce != transaction.nonce {
        return Err(ChainError::InvalidNonce {
            address: transaction.from.clone(),
            expected: sender.nonce,
            actual: transaction.nonce,
        });
    }
    if sender.balance < transaction.amount {
        return Err(ChainError::InsufficientFunds {
            address: transaction.from.clone(),
            balance: sender.balance,
            amount: transaction.amount,
        });
    }

    Ok(())
}

fn apply_transaction(
    accounts: &mut BTreeMap<String, Account>,
    transaction: &Transfer,
) -> Result<(), ChainError> {
    validate_transaction(accounts, transaction)?;

    let sender = accounts
        .get_mut(&transaction.from)
        .expect("validated sender must exist");
    sender.balance -= transaction.amount;
    sender.nonce += 1;

    let recipient = accounts
        .entry(transaction.to.clone())
        .or_insert_with(|| Account {
            address: transaction.to.clone(),
            balance: 0,
            nonce: 0,
        });
    recipient.balance = recipient
        .balance
        .checked_add(transaction.amount)
        .ok_or_else(|| ChainError::BalanceOverflow(transaction.to.clone()))?;

    Ok(())
}

fn pending_state(chain: &Chain) -> Result<BTreeMap<String, Account>, ChainError> {
    let mut accounts = chain.accounts.clone();
    for pending in &chain.pending_transactions {
        apply_transaction(&mut accounts, pending)?;
    }
    Ok(accounts)
}

fn ensure_genesis_finalized(chain: &Chain) -> Result<(), ChainError> {
    if chain.blocks.is_empty() {
        Err(ChainError::GenesisNotFinalized)
    } else {
        Ok(())
    }
}

fn accounts_from_genesis(
    genesis_accounts: &[Account],
) -> Result<BTreeMap<String, Account>, ChainError> {
    let mut accounts = BTreeMap::new();
    for account in genesis_accounts {
        if accounts
            .insert(account.address.clone(), account.clone())
            .is_some()
        {
            return Err(ChainError::DuplicateGenesisAccount(account.address.clone()));
        }
    }

    Ok(accounts)
}

fn transfer_payload(
    from: &str,
    to: &str,
    amount: u64,
    nonce: u64,
    public_key_base64: &str,
) -> String {
    serde_json::to_string(&UnsignedTransfer {
        from,
        to,
        amount,
        nonce,
        public_key_base64,
    })
    .expect("serializing an unsigned transfer should not fail")
}

fn block_hash(
    height: u64,
    parent_hash: &str,
    transactions: &[Transfer],
    state_root: &str,
) -> String {
    let payload = serde_json::to_vec(&BlockCommitment {
        height,
        parent_hash,
        transactions,
        state_root,
    })
    .expect("serializing a block commitment should not fail");
    encode_hex(&Sha256::digest(payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wallet_core::create_keypair;

    #[test]
    fn produces_and_validates_block() {
        let alice = create_keypair();
        let bob = create_keypair();
        let alice_address = address_from_public_key(&alice.public_key_base64).unwrap();
        let bob_address = address_from_public_key(&bob.public_key_base64).unwrap();
        let mut chain = new_chain(vec![Account {
            address: alice_address.clone(),
            balance: 100,
            nonce: 0,
        }])
        .unwrap();
        let genesis = finalize_genesis(&mut chain).unwrap();

        let transaction =
            sign_transfer(&alice.secret_key_base64, bob_address.clone(), 40, 0).unwrap();
        add_pending_transaction(&mut chain, transaction).unwrap();
        let block = produce_block(&mut chain).unwrap();

        assert_eq!(genesis.height, 0);
        assert_eq!(genesis.parent_hash, NULL_PARENT_HASH);
        assert_eq!(genesis.transactions, Vec::new());
        assert_eq!(block.height, 1);
        assert_eq!(block.parent_hash, genesis.block_hash);
        assert_eq!(chain.accounts[&alice_address].balance, 60);
        assert_eq!(chain.accounts[&alice_address].nonce, 1);
        assert_eq!(chain.accounts[&bob_address].balance, 40);
        assert_eq!(validate_chain(&chain), Ok(()));
    }

    #[test]
    fn rejects_replayed_nonce() {
        let alice = create_keypair();
        let bob = create_keypair();
        let alice_address = address_from_public_key(&alice.public_key_base64).unwrap();
        let bob_address = address_from_public_key(&bob.public_key_base64).unwrap();
        let mut chain = new_chain(vec![Account {
            address: alice_address.clone(),
            balance: 100,
            nonce: 0,
        }])
        .unwrap();
        finalize_genesis(&mut chain).unwrap();

        let first = sign_transfer(&alice.secret_key_base64, bob_address.clone(), 10, 0).unwrap();
        let replay = sign_transfer(&alice.secret_key_base64, bob_address, 10, 0).unwrap();

        assert_eq!(add_pending_transaction(&mut chain, first), Ok(()));
        assert_eq!(
            add_pending_transaction(&mut chain, replay),
            Err(ChainError::InvalidNonce {
                address: alice_address,
                expected: 1,
                actual: 0,
            })
        );
    }

    #[test]
    fn rejects_overspend() {
        let alice = create_keypair();
        let bob = create_keypair();
        let alice_address = address_from_public_key(&alice.public_key_base64).unwrap();
        let bob_address = address_from_public_key(&bob.public_key_base64).unwrap();
        let mut chain = new_chain(vec![Account {
            address: alice_address.clone(),
            balance: 25,
            nonce: 0,
        }])
        .unwrap();
        finalize_genesis(&mut chain).unwrap();
        let transaction = sign_transfer(&alice.secret_key_base64, bob_address, 26, 0).unwrap();

        assert_eq!(
            add_pending_transaction(&mut chain, transaction),
            Err(ChainError::InsufficientFunds {
                address: alice_address,
                balance: 25,
                amount: 26,
            })
        );
    }

    #[test]
    fn tampered_block_fails_validation() {
        let alice = create_keypair();
        let bob = create_keypair();
        let alice_address = address_from_public_key(&alice.public_key_base64).unwrap();
        let bob_address = address_from_public_key(&bob.public_key_base64).unwrap();
        let mut chain = new_chain(vec![Account {
            address: alice_address,
            balance: 100,
            nonce: 0,
        }])
        .unwrap();
        finalize_genesis(&mut chain).unwrap();
        let transaction = sign_transfer(&alice.secret_key_base64, bob_address, 40, 0).unwrap();
        add_pending_transaction(&mut chain, transaction).unwrap();
        produce_block(&mut chain).unwrap();

        chain.blocks[1].state_root =
            "0000000000000000000000000000000000000000000000000000000000000000".to_owned();

        assert_eq!(
            validate_chain(&chain),
            Err(ChainError::InvalidStateRoot { height: 1 })
        );
    }

    #[test]
    fn tampered_account_snapshot_fails_validation() {
        let alice = create_keypair();
        let bob = create_keypair();
        let alice_address = address_from_public_key(&alice.public_key_base64).unwrap();
        let bob_address = address_from_public_key(&bob.public_key_base64).unwrap();
        let mut chain = new_chain(vec![Account {
            address: alice_address.clone(),
            balance: 100,
            nonce: 0,
        }])
        .unwrap();
        finalize_genesis(&mut chain).unwrap();
        let transaction = sign_transfer(&alice.secret_key_base64, bob_address, 40, 0).unwrap();
        add_pending_transaction(&mut chain, transaction).unwrap();
        produce_block(&mut chain).unwrap();

        chain.accounts.get_mut(&alice_address).unwrap().balance = 100;

        assert_eq!(
            validate_chain(&chain),
            Err(ChainError::AccountSnapshotMismatch)
        );
    }

    #[test]
    fn requires_finalized_genesis_before_transactions() {
        let alice = create_keypair();
        let bob = create_keypair();
        let alice_address = address_from_public_key(&alice.public_key_base64).unwrap();
        let bob_address = address_from_public_key(&bob.public_key_base64).unwrap();
        let mut chain = new_chain(vec![Account {
            address: alice_address,
            balance: 100,
            nonce: 0,
        }])
        .unwrap();
        let transaction = sign_transfer(&alice.secret_key_base64, bob_address, 40, 0).unwrap();

        assert_eq!(
            add_pending_transaction(&mut chain, transaction),
            Err(ChainError::GenesisNotFinalized)
        );
        assert_eq!(
            produce_block(&mut chain),
            Err(ChainError::GenesisNotFinalized)
        );
    }
}
