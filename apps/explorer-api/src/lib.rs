use std::{fs, path::PathBuf, sync::Arc};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chain_core::{state_root, transaction_hash, validate_chain, Account, Block, Chain, Transfer};
use serde::Serialize;
use serde_json::json;

#[derive(Debug, Clone)]
struct AppState {
    chain_file: PathBuf,
}

#[derive(Debug, Serialize)]
struct ChainSummary {
    height: Option<u64>,
    block_count: usize,
    transaction_count: usize,
    pending_transaction_count: usize,
    account_count: usize,
    state_root: String,
    latest_block_hash: Option<String>,
    genesis_finalized: bool,
    valid: bool,
}

#[derive(Debug, Serialize)]
struct BlockSummary {
    height: u64,
    parent_hash: String,
    state_root: String,
    block_hash: String,
    transaction_count: usize,
}

#[derive(Debug, Serialize)]
struct TransactionView {
    hash: String,
    block_height: Option<u64>,
    transaction_index: usize,
    transfer: Transfer,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", content = "data")]
enum SearchResult {
    Block(Block),
    Account(Account),
    Transaction(TransactionView),
}

#[derive(Debug)]
enum ApiError {
    NotFound(String),
    ReadChain(String),
    ParseChain(String),
}

pub fn app(chain_file: PathBuf) -> Router {
    let state = Arc::new(AppState { chain_file });

    Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/chain/summary", get(chain_summary))
        .route("/blocks", get(blocks))
        .route("/blocks/:height", get(block_by_height))
        .route("/blocks/hash/:hash", get(block_by_hash))
        .route("/accounts/:address", get(account_by_address))
        .route("/transactions/:hash", get(transaction_by_hash))
        .route("/search/:query", get(search))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

async fn index() -> Json<serde_json::Value> {
    Json(json!({
        "name": "zk-rollup explorer api",
        "endpoints": [
            "GET /health",
            "GET /chain/summary",
            "GET /blocks",
            "GET /blocks/:height",
            "GET /blocks/hash/:hash",
            "GET /accounts/:address",
            "GET /transactions/:hash",
            "GET /search/:query"
        ]
    }))
}

async fn chain_summary(State(state): State<Arc<AppState>>) -> Result<Json<ChainSummary>, ApiError> {
    let chain = read_chain(&state.chain_file)?;
    let latest = chain.blocks.last();
    let summary = ChainSummary {
        height: latest.map(|block| block.height),
        block_count: chain.blocks.len(),
        transaction_count: chain
            .blocks
            .iter()
            .map(|block| block.transactions.len())
            .sum(),
        pending_transaction_count: chain.pending_transactions.len(),
        account_count: chain.accounts.len(),
        state_root: state_root(&chain.accounts),
        latest_block_hash: latest.map(|block| block.block_hash.clone()),
        genesis_finalized: !chain.blocks.is_empty(),
        valid: validate_chain(&chain).is_ok(),
    };

    Ok(Json(summary))
}

async fn blocks(State(state): State<Arc<AppState>>) -> Result<Json<Vec<BlockSummary>>, ApiError> {
    let chain = read_chain(&state.chain_file)?;
    let mut blocks = chain.blocks.iter().map(block_summary).collect::<Vec<_>>();
    blocks.reverse();
    Ok(Json(blocks))
}

async fn block_by_height(
    State(state): State<Arc<AppState>>,
    Path(height): Path<u64>,
) -> Result<Json<Block>, ApiError> {
    let chain = read_chain(&state.chain_file)?;
    find_block_by_height(&chain, height)
        .cloned()
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("block height {height} not found")))
}

async fn block_by_hash(
    State(state): State<Arc<AppState>>,
    Path(hash): Path<String>,
) -> Result<Json<Block>, ApiError> {
    let chain = read_chain(&state.chain_file)?;
    find_block_by_hash(&chain, &hash)
        .cloned()
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("block hash {hash} not found")))
}

async fn account_by_address(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> Result<Json<Account>, ApiError> {
    let chain = read_chain(&state.chain_file)?;
    chain
        .accounts
        .get(&address)
        .cloned()
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("account {address} not found")))
}

async fn transaction_by_hash(
    State(state): State<Arc<AppState>>,
    Path(hash): Path<String>,
) -> Result<Json<TransactionView>, ApiError> {
    let chain = read_chain(&state.chain_file)?;
    find_transaction_by_hash(&chain, &hash)
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("transaction {hash} not found")))
}

async fn search(
    State(state): State<Arc<AppState>>,
    Path(query): Path<String>,
) -> Result<Json<SearchResult>, ApiError> {
    let chain = read_chain(&state.chain_file)?;

    if let Some(block) = find_block_by_hash(&chain, &query) {
        return Ok(Json(SearchResult::Block(block.clone())));
    }
    if let Ok(height) = query.parse::<u64>() {
        if let Some(block) = find_block_by_height(&chain, height) {
            return Ok(Json(SearchResult::Block(block.clone())));
        }
    }
    if let Some(account) = chain.accounts.get(&query) {
        return Ok(Json(SearchResult::Account(account.clone())));
    }
    if let Some(transaction) = find_transaction_by_hash(&chain, &query) {
        return Ok(Json(SearchResult::Transaction(transaction)));
    }

    Err(ApiError::NotFound(format!("no result for {query}")))
}

fn read_chain(path: &PathBuf) -> Result<Chain, ApiError> {
    let data = fs::read_to_string(path)
        .map_err(|error| ApiError::ReadChain(format!("{}: {error}", path.display())))?;
    serde_json::from_str(&data)
        .map_err(|error| ApiError::ParseChain(format!("{}: {error}", path.display())))
}

fn block_summary(block: &Block) -> BlockSummary {
    BlockSummary {
        height: block.height,
        parent_hash: block.parent_hash.clone(),
        state_root: block.state_root.clone(),
        block_hash: block.block_hash.clone(),
        transaction_count: block.transactions.len(),
    }
}

fn find_block_by_height(chain: &Chain, height: u64) -> Option<&Block> {
    chain.blocks.iter().find(|block| block.height == height)
}

fn find_block_by_hash<'a>(chain: &'a Chain, hash: &str) -> Option<&'a Block> {
    chain.blocks.iter().find(|block| block.block_hash == hash)
}

fn find_transaction_by_hash(chain: &Chain, hash: &str) -> Option<TransactionView> {
    for block in &chain.blocks {
        for (transaction_index, transaction) in block.transactions.iter().enumerate() {
            let candidate = transaction_hash(transaction);
            if candidate == hash {
                return Some(TransactionView {
                    hash: candidate,
                    block_height: Some(block.height),
                    transaction_index,
                    transfer: transaction.clone(),
                });
            }
        }
    }

    for (transaction_index, transaction) in chain.pending_transactions.iter().enumerate() {
        let candidate = transaction_hash(transaction);
        if candidate == hash {
            return Some(TransactionView {
                hash: candidate,
                block_height: None,
                transaction_index,
                transfer: transaction.clone(),
            });
        }
    }

    None
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::NotFound(message) => (StatusCode::NOT_FOUND, message),
            ApiError::ReadChain(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
            ApiError::ParseChain(message) => (StatusCode::UNPROCESSABLE_ENTITY, message),
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}
