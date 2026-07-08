use std::collections::{BTreeMap, BTreeSet};

use merkle_core::{encode_hex, root_hex, SparseMerkleProof, SparseMerkleTree};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sha3::Keccak256;
use wallet_core::{
    address_from_secret, normalize_address, recover_address, sign_digest, WalletError,
};

const NULL_PARENT_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
pub const EIP712_DOMAIN_NAME: &str = "ZK Rollup";
pub const EIP712_DOMAIN_VERSION: &str = "1";
pub const ROLLUP_ID: &str = "zk-rollup-local-v1";
const TRANSACTION_HASH_DOMAIN: &str = "zk-rollup-transaction-v1";
const BLOCK_HASH_DOMAIN: &str = "zk-rollup-block-v1";
const BATCH_DATA_HASH_DOMAIN: &str = "zk-rollup-batch-data-v1";
const LOCAL_CHAIN_ID: &str = ROLLUP_ID;
const ACCOUNT_STATE_VALUE_DOMAIN: &[u8] = b"zk-rollup-account-state-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub address: String,
    pub balance: u64,
    pub nonce: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Transfer {
    pub from: String,
    pub to: String,
    pub amount: u64,
    pub nonce: u64,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct L1Deposit {
    pub chain_id: u64,
    pub inbox_address: String,
    pub deposit_id: u64,
    pub depositor_l1_address: String,
    pub recipient_l2_address: String,
    pub amount: u64,
    pub rolling_commitment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Block {
    pub height: u64,
    pub parent_hash: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deposits: Vec<L1Deposit>,
    pub transactions: Vec<Transfer>,
    pub state_root: String,
    pub block_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chain {
    pub genesis_accounts: Vec<Account>,
    pub accounts: BTreeMap<String, Account>,
    pub pending_transactions: Vec<Transfer>,
    pub blocks: Vec<Block>,
    #[serde(skip)]
    account_state_tree: SparseMerkleTree,
    #[serde(skip, default = "default_tree_dirty")]
    account_state_tree_dirty: bool,
}

impl PartialEq for Chain {
    fn eq(&self, other: &Self) -> bool {
        self.genesis_accounts == other.genesis_accounts
            && self.accounts == other.accounts
            && self.pending_transactions == other.pending_transactions
            && self.blocks == other.blocks
    }
}

impl Eq for Chain {}

fn default_tree_dirty() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RollupBatchCommitment {
    pub batch_number: u64,
    pub start_block_height: u64,
    pub end_block_height: u64,
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
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct BlockCommitment<'a> {
    domain: &'static str,
    chain_id: &'static str,
    height: u64,
    parent_hash: &'a str,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    deposits: &'a [L1Deposit],
    transactions: &'a [Transfer],
    state_root: &'a str,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct BatchDataCommitment<'a> {
    domain: &'static str,
    chain_id: &'static str,
    batch_number: u64,
    start_block_height: u64,
    end_block_height: u64,
    prev_state_root: &'a str,
    new_state_root: &'a str,
    blocks: &'a [Block],
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
    #[error("deposit amount must be greater than zero")]
    ZeroDepositAmount,
    #[error("sender account {0} does not exist")]
    MissingSender(String),
    #[error("invalid account address {0}")]
    InvalidAccountAddress(String),
    #[error("invalid deposit inbox address {0}")]
    InvalidDepositInboxAddress(String),
    #[error("invalid L1 depositor address {0}")]
    InvalidL1DepositorAddress(String),
    #[error("invalid rolling deposit commitment {0}")]
    InvalidDepositCommitment(String),
    #[error("duplicate deposit id {0}")]
    DuplicateDeposit(u64),
    #[error("deposit ids must be contiguous: expected {expected}, got {actual}")]
    NonContiguousDeposit { expected: u64, actual: u64 },
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
    #[error("rollup batch start height must be greater than zero")]
    InvalidBatchStart,
    #[error("rollup batch end height {end} is before start height {start}")]
    InvalidBatchRange { start: u64, end: u64 },
    #[error("rollup batch block height {height} does not exist")]
    MissingBatchBlock { height: u64 },
}

impl From<WalletError> for ChainError {
    fn from(error: WalletError) -> Self {
        Self::Wallet(error.to_string())
    }
}

pub fn new_chain(mut genesis_accounts: Vec<Account>) -> Result<Chain, ChainError> {
    for account in &mut genesis_accounts {
        account.address = normalize_address(&account.address)
            .map_err(|_| ChainError::InvalidAccountAddress(account.address.clone()))?;
    }
    let accounts = accounts_from_genesis(&genesis_accounts)?;
    let account_state_tree = account_state_tree(&accounts);

    Ok(Chain {
        genesis_accounts,
        accounts,
        pending_transactions: Vec::new(),
        blocks: Vec::new(),
        account_state_tree,
        account_state_tree_dirty: false,
    })
}

pub fn add_genesis_account(
    chain: &mut Chain,
    address: String,
    balance: u64,
) -> Result<(), ChainError> {
    let address = normalize_address(&address)
        .map_err(|_| ChainError::InvalidAccountAddress(address.clone()))?;
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
    chain
        .account_state_tree
        .update(address.as_bytes(), Some(&account_state_value(&account)));
    chain.accounts.insert(address, account);
    Ok(())
}

pub fn finalize_genesis(chain: &mut Chain) -> Result<Block, ChainError> {
    if !chain.blocks.is_empty() {
        return Err(ChainError::GenesisAlreadyFinalized);
    }

    ensure_account_state_tree(chain);
    let state_root = chain.account_state_tree.root_hex();
    let deposits = Vec::new();
    let transactions = Vec::new();
    let block_hash = block_hash(0, NULL_PARENT_HASH, &deposits, &transactions, &state_root);
    let block = Block {
        height: 0,
        parent_hash: NULL_PARENT_HASH.to_owned(),
        deposits,
        transactions,
        state_root,
        block_hash,
    };

    chain.blocks.push(block.clone());
    Ok(block)
}

pub fn sign_transfer(
    secret_key_hex: &str,
    to: String,
    amount: u64,
    nonce: u64,
) -> Result<Transfer, ChainError> {
    let from = address_from_secret(secret_key_hex)?;
    let to = normalize_address(&to)?;
    let digest = transfer_eip712_digest(&from, &to, amount, nonce)?;
    let signature = sign_digest(secret_key_hex, &digest)?;

    Ok(Transfer {
        from,
        to,
        amount,
        nonce,
        signature,
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
    ensure_account_state_tree(chain);
    let mut next_accounts = chain.accounts.clone();
    let deposits = Vec::new();
    for transaction in &chain.pending_transactions {
        apply_transaction_to_state(&mut next_accounts, transaction)?;
    }
    update_account_state_tree(
        &mut chain.account_state_tree,
        &next_accounts,
        &chain.pending_transactions,
    );

    let state_root = chain.account_state_tree.root_hex();
    let height = chain.blocks.len() as u64;
    let parent_hash = chain
        .blocks
        .last()
        .map(|block| block.block_hash.clone())
        .expect("genesis finalization ensures at least one block")
        .clone();
    let transactions = std::mem::take(&mut chain.pending_transactions);
    let block_hash = block_hash(height, &parent_hash, &deposits, &transactions, &state_root);

    let block = Block {
        height,
        parent_hash,
        deposits,
        transactions,
        state_root,
        block_hash,
    };

    chain.accounts = next_accounts;
    chain.account_state_tree_dirty = false;
    chain.blocks.push(block.clone());
    Ok(block)
}

pub fn produce_block_from_indexed_state(
    accounts: &mut BTreeMap<String, Account>,
    parent_block: &Block,
    transactions: Vec<Transfer>,
) -> Result<Block, ChainError> {
    produce_block_from_indexed_state_with_deposits(accounts, parent_block, Vec::new(), transactions)
}

pub fn produce_block_from_indexed_state_with_deposits(
    accounts: &mut BTreeMap<String, Account>,
    parent_block: &Block,
    deposits: Vec<L1Deposit>,
    transactions: Vec<Transfer>,
) -> Result<Block, ChainError> {
    let mut next_accounts = accounts.clone();
    validate_deposit_sequence(&deposits)?;
    for deposit in &deposits {
        apply_deposit_to_state(&mut next_accounts, deposit)?;
    }
    for transaction in &transactions {
        apply_transaction_to_state(&mut next_accounts, transaction)?;
    }

    let state_root = state_root(&next_accounts);
    let height = parent_block.height + 1;
    let block_hash = block_hash(
        height,
        &parent_block.block_hash,
        &deposits,
        &transactions,
        &state_root,
    );
    let block = Block {
        height,
        parent_hash: parent_block.block_hash.clone(),
        deposits,
        transactions,
        state_root,
        block_hash,
    };

    *accounts = next_accounts;
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
            validate_deposit_sequence(&block.deposits)?;
            for deposit in &block.deposits {
                apply_deposit_to_state(&mut accounts, deposit)?;
            }
            for transaction in &block.transactions {
                apply_transaction_to_state(&mut accounts, transaction)?;
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
            &block.deposits,
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
    account_state_tree(accounts).root_hex()
}

pub fn account_state_proof(
    accounts: &BTreeMap<String, Account>,
    address: &str,
) -> SparseMerkleProof {
    account_state_tree(accounts).prove(address.as_bytes())
}

pub fn account_state_value(account: &Account) -> Vec<u8> {
    let mut value = Vec::with_capacity(ACCOUNT_STATE_VALUE_DOMAIN.len() + 16);
    value.extend_from_slice(ACCOUNT_STATE_VALUE_DOMAIN);
    value.extend_from_slice(&account.balance.to_be_bytes());
    value.extend_from_slice(&account.nonce.to_be_bytes());
    value
}

fn account_state_tree(accounts: &BTreeMap<String, Account>) -> SparseMerkleTree {
    let entries = accounts
        .iter()
        .map(|(address, account)| (address.as_bytes(), account_state_value(account)))
        .collect::<Vec<_>>();
    SparseMerkleTree::new(entries)
        .expect("account map keys must produce unique sparse Merkle paths")
}

fn ensure_account_state_tree(chain: &mut Chain) {
    if chain.account_state_tree_dirty {
        chain.account_state_tree = account_state_tree(&chain.accounts);
        chain.account_state_tree_dirty = false;
    }
}

fn update_account_state_tree(
    tree: &mut SparseMerkleTree,
    accounts: &BTreeMap<String, Account>,
    transactions: &[Transfer],
) {
    let changed_addresses: BTreeSet<&str> = transactions
        .iter()
        .flat_map(|transaction| [transaction.from.as_str(), transaction.to.as_str()])
        .collect();
    for address in changed_addresses {
        let account = accounts
            .get(address)
            .expect("applied transaction accounts must exist");
        tree.update(address.as_bytes(), Some(&account_state_value(account)));
    }
}

pub fn transaction_hash(transaction: &Transfer) -> String {
    let payload = serde_json::to_vec(&json!({
        "domain": TRANSACTION_HASH_DOMAIN,
        "chain_id": LOCAL_CHAIN_ID,
        "transfer": transaction,
    }))
    .expect("serializing a transfer commitment should not fail");
    encode_hex(&Sha256::digest(payload))
}

pub fn rollup_batch_commitment(
    chain: &Chain,
    batch_number: u64,
    start_block_height: u64,
    end_block_height: u64,
) -> Result<RollupBatchCommitment, ChainError> {
    validate_chain(chain)?;

    if start_block_height == 0 {
        return Err(ChainError::InvalidBatchStart);
    }
    if end_block_height < start_block_height {
        return Err(ChainError::InvalidBatchRange {
            start: start_block_height,
            end: end_block_height,
        });
    }

    let previous_block = block_by_height(chain, start_block_height - 1)?;
    let blocks = (start_block_height..=end_block_height)
        .map(|height| block_by_height(chain, height).cloned())
        .collect::<Result<Vec<_>, _>>()?;
    let last_block = blocks
        .last()
        .expect("valid batch range must produce at least one block");

    let transaction_leaves =
        blocks
            .iter()
            .flat_map(|block| {
                block.transactions.iter().enumerate().map(
                    move |(transaction_index, transaction)| {
                        json!({
                            "block_height": block.height,
                            "transaction_index": transaction_index,
                            "transaction_hash": transaction_hash(transaction),
                            "transfer": transaction,
                        })
                    },
                )
            })
            .collect::<Vec<_>>();
    let deposit_leaves = deposit_leaves(&blocks);
    let block_hash_leaves = blocks
        .iter()
        .map(|block| {
            json!({
                "height": block.height,
                "block_hash": block.block_hash,
                "state_root": block.state_root,
            })
        })
        .collect::<Vec<_>>();
    let data = BatchDataCommitment {
        domain: BATCH_DATA_HASH_DOMAIN,
        chain_id: LOCAL_CHAIN_ID,
        batch_number,
        start_block_height,
        end_block_height,
        prev_state_root: &previous_block.state_root,
        new_state_root: &last_block.state_root,
        blocks: &blocks,
    };
    let data_payload =
        serde_json::to_vec(&data).expect("serializing a rollup batch should not fail");

    Ok(RollupBatchCommitment {
        batch_number,
        start_block_height,
        end_block_height,
        prev_state_root: previous_block.state_root.clone(),
        new_state_root: last_block.state_root.clone(),
        transaction_root: root_hex(&transaction_leaves),
        block_hash_root: root_hex(&block_hash_leaves),
        data_hash: encode_hex(&Sha256::digest(data_payload)),
        transaction_count: transaction_leaves.len(),
        deposit_count: deposit_leaves.len(),
        deposit_root: root_hex(&deposit_leaves),
        first_deposit_id: first_deposit_id(&deposit_leaves),
        last_deposit_id: last_deposit_id(&deposit_leaves),
    })
}

pub fn rollup_batch_commitment_from_blocks(
    batch_number: u64,
    previous_block: &Block,
    blocks: &[Block],
) -> Result<RollupBatchCommitment, ChainError> {
    if previous_block.height == 0 && blocks.first().is_none() {
        return Err(ChainError::InvalidBatchRange { start: 1, end: 0 });
    }

    let first_block = blocks.first().ok_or(ChainError::InvalidBatchRange {
        start: previous_block.height + 1,
        end: previous_block.height,
    })?;
    if first_block.height == 0 {
        return Err(ChainError::InvalidBatchStart);
    }
    if first_block.height != previous_block.height + 1 {
        return Err(ChainError::InvalidHeight {
            expected: previous_block.height + 1,
            actual: first_block.height,
        });
    }

    let mut expected_parent = previous_block.block_hash.clone();
    for (index, block) in blocks.iter().enumerate() {
        let expected_height = first_block.height + index as u64;
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
        let expected_block_hash = block_hash(
            block.height,
            &block.parent_hash,
            &block.deposits,
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

    let last_block = blocks
        .last()
        .expect("non-empty block slice checked before last block lookup");
    let transaction_leaves =
        blocks
            .iter()
            .flat_map(|block| {
                block.transactions.iter().enumerate().map(
                    move |(transaction_index, transaction)| {
                        json!({
                            "block_height": block.height,
                            "transaction_index": transaction_index,
                            "transaction_hash": transaction_hash(transaction),
                            "transfer": transaction,
                        })
                    },
                )
            })
            .collect::<Vec<_>>();
    let deposit_leaves = deposit_leaves(blocks);
    let block_hash_leaves = blocks
        .iter()
        .map(|block| {
            json!({
                "height": block.height,
                "block_hash": block.block_hash,
                "state_root": block.state_root,
            })
        })
        .collect::<Vec<_>>();
    let data = BatchDataCommitment {
        domain: BATCH_DATA_HASH_DOMAIN,
        chain_id: LOCAL_CHAIN_ID,
        batch_number,
        start_block_height: first_block.height,
        end_block_height: last_block.height,
        prev_state_root: &previous_block.state_root,
        new_state_root: &last_block.state_root,
        blocks,
    };
    let data_payload =
        serde_json::to_vec(&data).expect("serializing a rollup batch should not fail");

    Ok(RollupBatchCommitment {
        batch_number,
        start_block_height: first_block.height,
        end_block_height: last_block.height,
        prev_state_root: previous_block.state_root.clone(),
        new_state_root: last_block.state_root.clone(),
        transaction_root: root_hex(&transaction_leaves),
        block_hash_root: root_hex(&block_hash_leaves),
        data_hash: encode_hex(&Sha256::digest(data_payload)),
        transaction_count: transaction_leaves.len(),
        deposit_count: deposit_leaves.len(),
        deposit_root: root_hex(&deposit_leaves),
        first_deposit_id: first_deposit_id(&deposit_leaves),
        last_deposit_id: last_deposit_id(&deposit_leaves),
    })
}

pub fn validate_transaction(
    accounts: &BTreeMap<String, Account>,
    transaction: &Transfer,
) -> Result<(), ChainError> {
    if transaction.amount == 0 {
        return Err(ChainError::ZeroAmount);
    }

    let from = normalize_address(&transaction.from)?;
    let to = normalize_address(&transaction.to)?;
    if from != transaction.from || to != transaction.to {
        return Err(ChainError::InvalidAccountAddress(
            if from != transaction.from {
                transaction.from.clone()
            } else {
                transaction.to.clone()
            },
        ));
    }
    let digest = transfer_eip712_digest(
        &transaction.from,
        &transaction.to,
        transaction.amount,
        transaction.nonce,
    )?;
    if recover_address(&digest, &transaction.signature)? != transaction.from {
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

pub fn validate_deposit(deposit: &L1Deposit) -> Result<(), ChainError> {
    let inbox = normalize_address(&deposit.inbox_address)
        .map_err(|_| ChainError::InvalidDepositInboxAddress(deposit.inbox_address.clone()))?;
    if inbox != deposit.inbox_address {
        return Err(ChainError::InvalidDepositInboxAddress(
            deposit.inbox_address.clone(),
        ));
    }
    let depositor = normalize_address(&deposit.depositor_l1_address)
        .map_err(|_| ChainError::InvalidL1DepositorAddress(deposit.depositor_l1_address.clone()))?;
    if depositor != deposit.depositor_l1_address {
        return Err(ChainError::InvalidL1DepositorAddress(
            deposit.depositor_l1_address.clone(),
        ));
    }
    let recipient = normalize_address(&deposit.recipient_l2_address)
        .map_err(|_| ChainError::InvalidAccountAddress(deposit.recipient_l2_address.clone()))?;
    if recipient != deposit.recipient_l2_address {
        return Err(ChainError::InvalidAccountAddress(
            deposit.recipient_l2_address.clone(),
        ));
    }
    if deposit.amount == 0 {
        return Err(ChainError::ZeroDepositAmount);
    }
    let commitment = deposit
        .rolling_commitment
        .strip_prefix("0x")
        .ok_or_else(|| ChainError::InvalidDepositCommitment(deposit.rolling_commitment.clone()))?;
    if commitment.len() != 64 || !commitment.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ChainError::InvalidDepositCommitment(
            deposit.rolling_commitment.clone(),
        ));
    }
    Ok(())
}

pub fn validate_deposit_sequence(deposits: &[L1Deposit]) -> Result<(), ChainError> {
    let mut seen = BTreeSet::new();
    let mut expected = deposits.first().map(|deposit| deposit.deposit_id);
    for deposit in deposits {
        validate_deposit(deposit)?;
        if !seen.insert(deposit.deposit_id) {
            return Err(ChainError::DuplicateDeposit(deposit.deposit_id));
        }
        if let Some(expected_id) = expected {
            if deposit.deposit_id != expected_id {
                return Err(ChainError::NonContiguousDeposit {
                    expected: expected_id,
                    actual: deposit.deposit_id,
                });
            }
            expected = Some(expected_id + 1);
        }
    }
    Ok(())
}

pub fn apply_transaction_to_state(
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

pub fn apply_deposit_to_state(
    accounts: &mut BTreeMap<String, Account>,
    deposit: &L1Deposit,
) -> Result<(), ChainError> {
    validate_deposit(deposit)?;
    let recipient = accounts
        .entry(deposit.recipient_l2_address.clone())
        .or_insert_with(|| Account {
            address: deposit.recipient_l2_address.clone(),
            balance: 0,
            nonce: 0,
        });
    recipient.balance = recipient
        .balance
        .checked_add(deposit.amount)
        .ok_or_else(|| ChainError::BalanceOverflow(deposit.recipient_l2_address.clone()))?;
    Ok(())
}

fn pending_state(chain: &Chain) -> Result<BTreeMap<String, Account>, ChainError> {
    let mut accounts = chain.accounts.clone();
    for pending in &chain.pending_transactions {
        apply_transaction_to_state(&mut accounts, pending)?;
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

fn block_by_height(chain: &Chain, height: u64) -> Result<&Block, ChainError> {
    chain
        .blocks
        .iter()
        .find(|block| block.height == height)
        .ok_or(ChainError::MissingBatchBlock { height })
}

pub fn transfer_eip712_digest(
    from: &str,
    to: &str,
    amount: u64,
    nonce: u64,
) -> Result<[u8; 32], ChainError> {
    let domain_type = keccak(b"EIP712Domain(string name,string version)");
    let transfer_type =
        keccak(b"Transfer(address from,address to,uint256 amount,uint256 nonce,string rollupId)");
    let domain_separator = keccak_words(&[
        domain_type,
        keccak(EIP712_DOMAIN_NAME.as_bytes()),
        keccak(EIP712_DOMAIN_VERSION.as_bytes()),
    ]);
    let transfer_hash = keccak_words(&[
        transfer_type,
        address_word(from)?,
        address_word(to)?,
        uint_word(amount),
        uint_word(nonce),
        keccak(ROLLUP_ID.as_bytes()),
    ]);
    let mut encoded = Vec::with_capacity(66);
    encoded.extend_from_slice(b"\x19\x01");
    encoded.extend_from_slice(&domain_separator);
    encoded.extend_from_slice(&transfer_hash);
    Ok(keccak(&encoded))
}

fn keccak(value: &[u8]) -> [u8; 32] {
    Keccak256::digest(value).into()
}

fn keccak_words(words: &[[u8; 32]]) -> [u8; 32] {
    let mut encoded = Vec::with_capacity(words.len() * 32);
    for word in words {
        encoded.extend_from_slice(word);
    }
    keccak(&encoded)
}

fn address_word(address: &str) -> Result<[u8; 32], ChainError> {
    let normalized = normalize_address(address)?;
    let bytes = hex::decode(&normalized[2..])
        .map_err(|_| ChainError::InvalidAccountAddress(address.to_owned()))?;
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(&bytes);
    Ok(word)
}

fn uint_word(value: u64) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    word
}

fn deposit_leaves(blocks: &[Block]) -> Vec<serde_json::Value> {
    blocks
        .iter()
        .flat_map(|block| {
            block
                .deposits
                .iter()
                .enumerate()
                .map(move |(deposit_index, deposit)| {
                    json!({
                        "block_height": block.height,
                        "deposit_index": deposit_index,
                        "deposit": deposit,
                    })
                })
        })
        .collect()
}

fn first_deposit_id(deposit_leaves: &[serde_json::Value]) -> Option<u64> {
    deposit_leaves
        .first()
        .and_then(|leaf| leaf["deposit"]["deposit_id"].as_u64())
}

fn last_deposit_id(deposit_leaves: &[serde_json::Value]) -> Option<u64> {
    deposit_leaves
        .last()
        .and_then(|leaf| leaf["deposit"]["deposit_id"].as_u64())
}

fn block_hash(
    height: u64,
    parent_hash: &str,
    deposits: &[L1Deposit],
    transactions: &[Transfer],
    state_root: &str,
) -> String {
    let payload = serde_json::to_vec(&BlockCommitment {
        domain: BLOCK_HASH_DOMAIN,
        chain_id: LOCAL_CHAIN_ID,
        height,
        parent_hash,
        deposits,
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
        let alice_address = alice.address.clone();
        let bob_address = bob.address.clone();
        let mut chain = new_chain(vec![Account {
            address: alice_address.clone(),
            balance: 100,
            nonce: 0,
        }])
        .unwrap();
        let genesis = finalize_genesis(&mut chain).unwrap();

        let transaction = sign_transfer(&alice.secret_key_hex, bob_address.clone(), 40, 0).unwrap();
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
    fn indexed_deposits_credit_l2_state_before_transfers() {
        let alice = create_keypair();
        let bob = create_keypair();
        let alice_address = alice.address.clone();
        let bob_address = bob.address.clone();
        let mut chain = new_chain(Vec::new()).unwrap();
        let genesis = finalize_genesis(&mut chain).unwrap();
        let mut accounts = chain.accounts.clone();
        let deposit = L1Deposit {
            chain_id: 31337,
            inbox_address: "0x2222222222222222222222222222222222222222".to_owned(),
            deposit_id: 0,
            depositor_l1_address: "0x3333333333333333333333333333333333333333".to_owned(),
            recipient_l2_address: alice_address.clone(),
            amount: 100,
            rolling_commitment: format!("0x{}", "4".repeat(64)),
        };
        let transfer = sign_transfer(&alice.secret_key_hex, bob_address.clone(), 40, 0).unwrap();

        let block = produce_block_from_indexed_state_with_deposits(
            &mut accounts,
            &genesis,
            vec![deposit.clone()],
            vec![transfer],
        )
        .unwrap();

        assert_eq!(block.deposits, vec![deposit]);
        assert_eq!(accounts[&alice_address].balance, 60);
        assert_eq!(accounts[&alice_address].nonce, 1);
        assert_eq!(accounts[&bob_address].balance, 40);
        assert_eq!(block.state_root, state_root(&accounts));
    }

    #[test]
    fn rejects_non_contiguous_deposit_ids() {
        let mut accounts = BTreeMap::new();
        let deposit = L1Deposit {
            chain_id: 31337,
            inbox_address: "0x2222222222222222222222222222222222222222".to_owned(),
            deposit_id: 2,
            depositor_l1_address: "0x3333333333333333333333333333333333333333".to_owned(),
            recipient_l2_address: "0x4444444444444444444444444444444444444444".to_owned(),
            amount: 100,
            rolling_commitment: format!("0x{}", "5".repeat(64)),
        };
        let mut skipped = deposit.clone();
        skipped.deposit_id = 4;
        let mut chain = new_chain(Vec::new()).unwrap();
        let genesis = finalize_genesis(&mut chain).unwrap();

        assert_eq!(
            produce_block_from_indexed_state_with_deposits(
                &mut accounts,
                &genesis,
                vec![deposit, skipped],
                Vec::new(),
            ),
            Err(ChainError::NonContiguousDeposit {
                expected: 3,
                actual: 4
            })
        );
    }

    #[test]
    fn rejects_replayed_nonce() {
        let alice = create_keypair();
        let bob = create_keypair();
        let alice_address = alice.address.clone();
        let bob_address = bob.address.clone();
        let mut chain = new_chain(vec![Account {
            address: alice_address.clone(),
            balance: 100,
            nonce: 0,
        }])
        .unwrap();
        finalize_genesis(&mut chain).unwrap();

        let first = sign_transfer(&alice.secret_key_hex, bob_address.clone(), 10, 0).unwrap();
        let replay = sign_transfer(&alice.secret_key_hex, bob_address, 10, 0).unwrap();

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
    fn rejects_signature_from_wrong_protocol_domain() {
        let alice = create_keypair();
        let bob = create_keypair();
        let alice_address = alice.address.clone();
        let bob_address = bob.address.clone();
        let mut chain = new_chain(vec![Account {
            address: alice_address.clone(),
            balance: 100,
            nonce: 0,
        }])
        .unwrap();
        finalize_genesis(&mut chain).unwrap();

        let wrong_digest: [u8; 32] = Keccak256::digest(b"different-protocol").into();
        let transaction = Transfer {
            from: alice.address,
            to: bob_address,
            amount: 10,
            nonce: 0,
            signature: wallet_core::sign_digest(&alice.secret_key_hex, &wrong_digest).unwrap(),
        };

        assert_eq!(
            add_pending_transaction(&mut chain, transaction),
            Err(ChainError::InvalidSignature)
        );
    }

    #[test]
    fn verifies_eip712_signature_from_an_ethereum_wallet() {
        let from = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";
        let to = "0x2222222222222222222222222222222222222222";
        let signature = "0x141c9f80090f93ff33a2a7a439c6f186277e37b245b0595d9a7e96cef8b83df279af32f70c663a4f7fc63ae49936066bd4f11f551a95559196910c3c005fc25c1c";
        let transaction = Transfer {
            from: from.to_owned(),
            to: to.to_owned(),
            amount: 12,
            nonce: 3,
            signature: signature.to_owned(),
        };
        let accounts = BTreeMap::from([(
            from.to_owned(),
            Account {
                address: from.to_owned(),
                balance: 20,
                nonce: 3,
            },
        )]);

        assert_eq!(validate_transaction(&accounts, &transaction), Ok(()));
    }

    #[test]
    fn rejects_overspend() {
        let alice = create_keypair();
        let bob = create_keypair();
        let alice_address = alice.address.clone();
        let bob_address = bob.address.clone();
        let mut chain = new_chain(vec![Account {
            address: alice_address.clone(),
            balance: 25,
            nonce: 0,
        }])
        .unwrap();
        finalize_genesis(&mut chain).unwrap();
        let transaction = sign_transfer(&alice.secret_key_hex, bob_address, 26, 0).unwrap();

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
        let alice_address = alice.address.clone();
        let bob_address = bob.address.clone();
        let mut chain = new_chain(vec![Account {
            address: alice_address,
            balance: 100,
            nonce: 0,
        }])
        .unwrap();
        finalize_genesis(&mut chain).unwrap();
        let transaction = sign_transfer(&alice.secret_key_hex, bob_address, 40, 0).unwrap();
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
        let alice_address = alice.address.clone();
        let bob_address = bob.address.clone();
        let mut chain = new_chain(vec![Account {
            address: alice_address.clone(),
            balance: 100,
            nonce: 0,
        }])
        .unwrap();
        finalize_genesis(&mut chain).unwrap();
        let transaction = sign_transfer(&alice.secret_key_hex, bob_address, 40, 0).unwrap();
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
        let alice_address = alice.address.clone();
        let bob_address = bob.address.clone();
        let mut chain = new_chain(vec![Account {
            address: alice_address,
            balance: 100,
            nonce: 0,
        }])
        .unwrap();
        let transaction = sign_transfer(&alice.secret_key_hex, bob_address, 40, 0).unwrap();

        assert_eq!(
            add_pending_transaction(&mut chain, transaction),
            Err(ChainError::GenesisNotFinalized)
        );
        assert_eq!(
            produce_block(&mut chain),
            Err(ChainError::GenesisNotFinalized)
        );
    }

    #[test]
    fn rejects_unknown_chain_json_fields() {
        let result = serde_json::from_value::<Chain>(json!({
            "genesis_accounts": [],
            "accounts": {},
            "pending_transactions": [],
            "blocks": [],
            "unexpected": true
        }));

        assert!(result.is_err());
    }

    #[test]
    fn loaded_chain_rebuilds_tree_once_then_updates_it_incrementally() {
        let alice = create_keypair();
        let bob = create_keypair();
        let alice_address = alice.address.clone();
        let bob_address = bob.address.clone();
        let mut chain = new_chain(vec![Account {
            address: alice_address.clone(),
            balance: 100,
            nonce: 0,
        }])
        .unwrap();
        finalize_genesis(&mut chain).unwrap();

        let first = sign_transfer(&alice.secret_key_hex, bob_address.clone(), 20, 0).unwrap();
        add_pending_transaction(&mut chain, first).unwrap();
        produce_block(&mut chain).unwrap();

        let json = serde_json::to_string(&chain).unwrap();
        assert!(!json.contains("account_state_tree"));
        let mut loaded: Chain = serde_json::from_str(&json).unwrap();

        let second = sign_transfer(&alice.secret_key_hex, bob_address, 15, 1).unwrap();
        add_pending_transaction(&mut loaded, second).unwrap();
        let block = produce_block(&mut loaded).unwrap();

        assert_eq!(block.state_root, state_root(&loaded.accounts));
        validate_chain(&loaded).unwrap();
    }

    #[test]
    fn builds_rollup_batch_commitment_from_block_range() {
        let alice = create_keypair();
        let bob = create_keypair();
        let alice_address = alice.address.clone();
        let bob_address = bob.address.clone();
        let mut chain = new_chain(vec![Account {
            address: alice_address,
            balance: 100,
            nonce: 0,
        }])
        .unwrap();
        let genesis = finalize_genesis(&mut chain).unwrap();

        let first = sign_transfer(&alice.secret_key_hex, bob_address.clone(), 25, 0).unwrap();
        add_pending_transaction(&mut chain, first).unwrap();
        let block = produce_block(&mut chain).unwrap();

        let batch = rollup_batch_commitment(&chain, 0, 1, 1).unwrap();

        assert_eq!(batch.batch_number, 0);
        assert_eq!(batch.start_block_height, 1);
        assert_eq!(batch.end_block_height, 1);
        assert_eq!(batch.prev_state_root, genesis.state_root);
        assert_eq!(batch.new_state_root, block.state_root);
        assert_eq!(batch.transaction_count, 1);
        assert_eq!(batch.transaction_root.len(), 64);
        assert_eq!(batch.block_hash_root.len(), 64);
        assert_eq!(batch.data_hash.len(), 64);
    }

    #[test]
    fn rejects_rollup_batch_that_starts_at_genesis() {
        let chain = new_chain(Vec::new()).unwrap();

        assert_eq!(
            rollup_batch_commitment(&chain, 0, 0, 0),
            Err(ChainError::InvalidBatchStart)
        );
    }

    #[test]
    fn account_state_supports_inclusion_and_non_membership_proofs() {
        let account = Account {
            address: "alice".to_owned(),
            balance: 100,
            nonce: 3,
        };
        let accounts = BTreeMap::from([("alice".to_owned(), account.clone())]);
        let root = state_root(&accounts);

        assert!(merkle_core::verify_sparse_proof(
            b"alice",
            Some(&account_state_value(&account)),
            &account_state_proof(&accounts, "alice"),
            &root,
        )
        .unwrap());
        assert!(merkle_core::verify_sparse_proof(
            b"bob",
            None,
            &account_state_proof(&accounts, "bob"),
            &root,
        )
        .unwrap());
    }

    #[test]
    fn rejects_rollup_batch_missing_block() {
        let mut chain = new_chain(Vec::new()).unwrap();
        finalize_genesis(&mut chain).unwrap();

        assert_eq!(
            rollup_batch_commitment(&chain, 0, 1, 1),
            Err(ChainError::MissingBatchBlock { height: 1 })
        );
    }
}
