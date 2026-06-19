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
use sqlx::{PgPool, Row};

#[derive(Debug, Clone)]
struct AppState {
    source: ChainSource,
}

#[derive(Debug, Clone)]
enum ChainSource {
    Json { chain_file: PathBuf },
    Postgres { pool: PgPool },
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
    Database(String),
    InvalidIndexedData(String),
}

pub fn app(chain_file: PathBuf) -> Router {
    app_with_source(ChainSource::Json { chain_file })
}

pub fn app_with_pool(pool: PgPool) -> Router {
    app_with_source(ChainSource::Postgres { pool })
}

fn app_with_source(source: ChainSource) -> Router {
    let state = Arc::new(AppState { source });

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
    state.source.chain_summary().await.map(Json)
}

async fn blocks(State(state): State<Arc<AppState>>) -> Result<Json<Vec<BlockSummary>>, ApiError> {
    state.source.blocks().await.map(Json)
}

async fn block_by_height(
    State(state): State<Arc<AppState>>,
    Path(height): Path<u64>,
) -> Result<Json<Block>, ApiError> {
    state
        .source
        .block_by_height(height)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("block height {height} not found")))
}

async fn block_by_hash(
    State(state): State<Arc<AppState>>,
    Path(hash): Path<String>,
) -> Result<Json<Block>, ApiError> {
    state
        .source
        .block_by_hash(&hash)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("block hash {hash} not found")))
}

async fn account_by_address(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> Result<Json<Account>, ApiError> {
    state
        .source
        .account_by_address(&address)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("account {address} not found")))
}

async fn transaction_by_hash(
    State(state): State<Arc<AppState>>,
    Path(hash): Path<String>,
) -> Result<Json<TransactionView>, ApiError> {
    state
        .source
        .transaction_by_hash(&hash)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("transaction {hash} not found")))
}

async fn search(
    State(state): State<Arc<AppState>>,
    Path(query): Path<String>,
) -> Result<Json<SearchResult>, ApiError> {
    if let Some(block) = state.source.block_by_hash(&query).await? {
        return Ok(Json(SearchResult::Block(block)));
    }
    if let Ok(height) = query.parse::<u64>() {
        if let Some(block) = state.source.block_by_height(height).await? {
            return Ok(Json(SearchResult::Block(block)));
        }
    }
    if let Some(account) = state.source.account_by_address(&query).await? {
        return Ok(Json(SearchResult::Account(account)));
    }
    if let Some(transaction) = state.source.transaction_by_hash(&query).await? {
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

impl ChainSource {
    async fn chain_summary(&self) -> Result<ChainSummary, ApiError> {
        match self {
            ChainSource::Json { chain_file } => {
                let chain = read_chain(chain_file)?;
                Ok(json_chain_summary(&chain))
            }
            ChainSource::Postgres { pool } => postgres_chain_summary(pool).await,
        }
    }

    async fn blocks(&self) -> Result<Vec<BlockSummary>, ApiError> {
        match self {
            ChainSource::Json { chain_file } => {
                let chain = read_chain(chain_file)?;
                let mut blocks = chain.blocks.iter().map(block_summary).collect::<Vec<_>>();
                blocks.reverse();
                Ok(blocks)
            }
            ChainSource::Postgres { pool } => {
                let rows = sqlx::query(
                    r#"
                    SELECT height, parent_hash, state_root, block_hash, transaction_count
                    FROM blocks
                    ORDER BY height DESC
                    "#,
                )
                .fetch_all(pool)
                .await
                .map_err(database_error)?;

                rows.into_iter().map(block_summary_from_row).collect()
            }
        }
    }

    async fn block_by_height(&self, height: u64) -> Result<Option<Block>, ApiError> {
        match self {
            ChainSource::Json { chain_file } => {
                let chain = read_chain(chain_file)?;
                Ok(find_block_by_height(&chain, height).cloned())
            }
            ChainSource::Postgres { pool } => {
                let row = sqlx::query("SELECT raw_block FROM blocks WHERE height = $1")
                    .bind(to_i64(height, "block height")?)
                    .fetch_optional(pool)
                    .await
                    .map_err(database_error)?;

                row.map(|row| parse_json_value(row.get("raw_block"), "block"))
                    .transpose()
            }
        }
    }

    async fn block_by_hash(&self, hash: &str) -> Result<Option<Block>, ApiError> {
        match self {
            ChainSource::Json { chain_file } => {
                let chain = read_chain(chain_file)?;
                Ok(find_block_by_hash(&chain, hash).cloned())
            }
            ChainSource::Postgres { pool } => {
                let row = sqlx::query("SELECT raw_block FROM blocks WHERE block_hash = $1")
                    .bind(hash)
                    .fetch_optional(pool)
                    .await
                    .map_err(database_error)?;

                row.map(|row| parse_json_value(row.get("raw_block"), "block"))
                    .transpose()
            }
        }
    }

    async fn account_by_address(&self, address: &str) -> Result<Option<Account>, ApiError> {
        match self {
            ChainSource::Json { chain_file } => {
                let chain = read_chain(chain_file)?;
                Ok(chain.accounts.get(address).cloned())
            }
            ChainSource::Postgres { pool } => {
                let row = sqlx::query(
                    r#"
                    SELECT address, balance, nonce
                    FROM accounts
                    WHERE address = $1
                    "#,
                )
                .bind(address)
                .fetch_optional(pool)
                .await
                .map_err(database_error)?;

                row.map(account_from_row).transpose()
            }
        }
    }

    async fn transaction_by_hash(&self, hash: &str) -> Result<Option<TransactionView>, ApiError> {
        match self {
            ChainSource::Json { chain_file } => {
                let chain = read_chain(chain_file)?;
                Ok(find_transaction_by_hash(&chain, hash))
            }
            ChainSource::Postgres { pool } => {
                let row = sqlx::query(
                    r#"
                    SELECT hash, block_height, transaction_index, raw_transfer
                    FROM transactions
                    WHERE hash = $1
                    "#,
                )
                .bind(hash)
                .fetch_optional(pool)
                .await
                .map_err(database_error)?;

                row.map(transaction_from_row).transpose()
            }
        }
    }
}

fn json_chain_summary(chain: &Chain) -> ChainSummary {
    let latest = chain.blocks.last();
    ChainSummary {
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
        valid: validate_chain(chain).is_ok(),
    }
}

async fn postgres_chain_summary(pool: &PgPool) -> Result<ChainSummary, ApiError> {
    let block_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blocks")
        .fetch_one(pool)
        .await
        .map_err(database_error)?;
    let transaction_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE block_height IS NOT NULL")
            .fetch_one(pool)
            .await
            .map_err(database_error)?;
    let pending_transaction_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE block_height IS NULL")
            .fetch_one(pool)
            .await
            .map_err(database_error)?;
    let account_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
        .fetch_one(pool)
        .await
        .map_err(database_error)?;
    let latest = sqlx::query(
        r#"
        SELECT height, state_root, block_hash
        FROM blocks
        ORDER BY height DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(pool)
    .await
    .map_err(database_error)?;
    let checkpoint_height: Option<i64> = sqlx::query_scalar(
        "SELECT last_indexed_height FROM indexer_checkpoints WHERE id = 'chain'",
    )
    .fetch_optional(pool)
    .await
    .map_err(database_error)?
    .flatten();

    let (height, state_root, latest_block_hash) = match latest {
        Some(row) => (
            Some(from_i64(row.get("height"), "block height")?),
            row.get("state_root"),
            Some(row.get("block_hash")),
        ),
        None => (None, state_root_from_accounts(pool).await?, None),
    };
    let checkpoint_matches = checkpoint_height
        .map(|checkpoint| Some(checkpoint) == height.map(|height| height as i64))
        .unwrap_or(block_count == 0);

    Ok(ChainSummary {
        height,
        block_count: usize_from_i64(block_count, "block count")?,
        transaction_count: usize_from_i64(transaction_count, "transaction count")?,
        pending_transaction_count: usize_from_i64(
            pending_transaction_count,
            "pending transaction count",
        )?,
        account_count: usize_from_i64(account_count, "account count")?,
        state_root,
        latest_block_hash,
        genesis_finalized: block_count > 0,
        valid: checkpoint_matches,
    })
}

async fn state_root_from_accounts(pool: &PgPool) -> Result<String, ApiError> {
    let rows = sqlx::query(
        r#"
        SELECT address, balance, nonce
        FROM accounts
        ORDER BY address ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(database_error)?;
    let accounts = rows
        .into_iter()
        .map(account_from_row)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|account| (account.address.clone(), account))
        .collect();

    Ok(state_root(&accounts))
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

fn block_summary_from_row(row: sqlx::postgres::PgRow) -> Result<BlockSummary, ApiError> {
    Ok(BlockSummary {
        height: from_i64(row.get("height"), "block height")?,
        parent_hash: row.get("parent_hash"),
        state_root: row.get("state_root"),
        block_hash: row.get("block_hash"),
        transaction_count: usize_from_i32(row.get("transaction_count"), "transaction count")?,
    })
}

fn account_from_row(row: sqlx::postgres::PgRow) -> Result<Account, ApiError> {
    Ok(Account {
        address: row.get("address"),
        balance: from_i64(row.get("balance"), "account balance")?,
        nonce: from_i64(row.get("nonce"), "account nonce")?,
    })
}

fn transaction_from_row(row: sqlx::postgres::PgRow) -> Result<TransactionView, ApiError> {
    Ok(TransactionView {
        hash: row.get("hash"),
        block_height: row
            .get::<Option<i64>, _>("block_height")
            .map(|height| from_i64(height, "transaction block height"))
            .transpose()?,
        transaction_index: usize_from_i32(row.get("transaction_index"), "transaction index")?,
        transfer: parse_json_value(row.get("raw_transfer"), "transaction")?,
    })
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

fn parse_json_value<T: serde::de::DeserializeOwned>(
    value: serde_json::Value,
    label: &str,
) -> Result<T, ApiError> {
    serde_json::from_value(value)
        .map_err(|error| ApiError::InvalidIndexedData(format!("invalid indexed {label}: {error}")))
}

fn database_error(error: sqlx::Error) -> ApiError {
    ApiError::Database(error.to_string())
}

fn to_i64(value: u64, label: &str) -> Result<i64, ApiError> {
    i64::try_from(value)
        .map_err(|_| ApiError::InvalidIndexedData(format!("{label} exceeds BIGINT range")))
}

fn from_i64(value: i64, label: &str) -> Result<u64, ApiError> {
    u64::try_from(value).map_err(|_| ApiError::InvalidIndexedData(format!("{label} is negative")))
}

fn usize_from_i64(value: i64, label: &str) -> Result<usize, ApiError> {
    usize::try_from(value)
        .map_err(|_| ApiError::InvalidIndexedData(format!("{label} exceeds usize range")))
}

fn usize_from_i32(value: i32, label: &str) -> Result<usize, ApiError> {
    usize::try_from(value).map_err(|_| ApiError::InvalidIndexedData(format!("{label} is negative")))
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::NotFound(message) => (StatusCode::NOT_FOUND, message),
            ApiError::ReadChain(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
            ApiError::ParseChain(message) => (StatusCode::UNPROCESSABLE_ENTITY, message),
            ApiError::Database(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
            ApiError::InvalidIndexedData(message) => (StatusCode::UNPROCESSABLE_ENTITY, message),
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}
