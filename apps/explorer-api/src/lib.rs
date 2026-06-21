use std::{
    collections::HashMap,
    fmt, fs,
    future::Future,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chain_core::{
    rollup_batch_commitment, state_root, transaction_hash, validate_chain, Account, Block, Chain,
    RollupBatchCommitment, Transfer,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use tower_http::cors::CorsLayer;

const POSTGRES_CACHE_TTL: Duration = Duration::from_secs(5);
const DEFAULT_PAGE_LIMIT: usize = 50;
const MAX_PAGE_LIMIT: usize = 100;

#[derive(Debug, Clone)]
struct AppState {
    source: ChainSource,
    cache: Arc<Mutex<ResponseCache>>,
}

#[derive(Debug, Default)]
struct ResponseCache {
    entries: HashMap<String, CacheEntry>,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    expires_at: Instant,
    value: Value,
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

#[derive(Debug, Deserialize)]
struct PageQuery {
    limit: Option<usize>,
    offset: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
struct PageParams {
    limit: usize,
    offset: usize,
}

#[derive(Debug, Serialize)]
struct Page<T> {
    items: Vec<T>,
    limit: usize,
    offset: usize,
    total: usize,
    next_offset: Option<usize>,
}

impl PageQuery {
    fn page_params(self) -> Result<PageParams, ApiError> {
        let limit = self.limit.unwrap_or(DEFAULT_PAGE_LIMIT);
        if limit == 0 {
            return Err(ApiError::BadRequest(
                "limit must be greater than zero".to_string(),
            ));
        }
        if limit > MAX_PAGE_LIMIT {
            return Err(ApiError::BadRequest(format!(
                "limit must be less than or equal to {MAX_PAGE_LIMIT}"
            )));
        }

        Ok(PageParams {
            limit,
            offset: self.offset.unwrap_or(0),
        })
    }
}

impl fmt::Display for PageParams {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "limit:{}:offset:{}", self.limit, self.offset)
    }
}

fn page_from_items<T>(items: Vec<T>, total: usize, page: PageParams) -> Page<T> {
    let consumed = page.offset.saturating_add(items.len());
    Page {
        items,
        limit: page.limit,
        offset: page.offset,
        total,
        next_offset: (consumed < total).then_some(consumed),
    }
}

#[derive(Debug, Serialize)]
struct TransactionView {
    hash: String,
    block_height: Option<u64>,
    transaction_index: usize,
    transfer: Transfer,
}

#[derive(Debug, Serialize)]
struct BlockView {
    height: u64,
    parent_hash: String,
    transactions: Vec<Transfer>,
    state_root: String,
    block_hash: String,
    transaction_views: Vec<TransactionView>,
}

#[derive(Debug, Serialize)]
struct ErrorEventView {
    id: i64,
    service: String,
    severity: String,
    message: String,
    context: Value,
    created_at: String,
    resolved_at: Option<String>,
}

#[derive(Debug, Serialize)]
struct RollupBatchSummary {
    batch_number: u64,
    start_block_height: u64,
    end_block_height: u64,
    prev_state_root: String,
    new_state_root: String,
    transaction_root: String,
    block_hash_root: String,
    data_hash: String,
    transaction_count: usize,
    status: String,
}

#[derive(Debug, Serialize)]
struct RollupBatchView {
    #[serde(flatten)]
    summary: RollupBatchSummary,
    transactions: Vec<TransactionView>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", content = "data")]
enum SearchResult {
    Block(BlockView),
    Account(Account),
    Transaction(TransactionView),
}

#[derive(Debug)]
enum ApiError {
    BadRequest(String),
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
    let state = Arc::new(AppState {
        source,
        cache: Arc::new(Mutex::new(ResponseCache::default())),
    });

    Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/chain/summary", get(chain_summary))
        .route("/blocks", get(blocks))
        .route("/blocks/:height", get(block_by_height))
        .route("/blocks/hash/:hash", get(block_by_hash))
        .route("/accounts/:address", get(account_by_address))
        .route("/transactions/:hash", get(transaction_by_hash))
        .route("/errors", get(errors))
        .route("/errors/latest", get(latest_error))
        .route("/rollup/batches", get(rollup_batches))
        .route("/rollup/batches/:batch_number", get(rollup_batch_by_number))
        .route("/search/:query", get(search))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn health(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let latest_error = state.source.latest_error().await?;
    let status = if latest_error.is_some() {
        "degraded"
    } else {
        "ok"
    };

    Ok(Json(json!({
        "status": status,
        "latest_error": latest_error,
    })))
}

async fn index() -> Json<Value> {
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
            "GET /errors",
            "GET /errors/latest",
            "GET /rollup/batches",
            "GET /rollup/batches/:batch_number",
            "GET /search/:query"
        ]
    }))
}

async fn chain_summary(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    cached_json(state, "chain:summary".to_string(), |source| async move {
        source.chain_summary().await
    })
    .await
}

async fn blocks(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PageQuery>,
) -> Result<Json<Value>, ApiError> {
    let page = query.page_params()?;
    cached_json(state, format!("blocks:{page}"), |source| async move {
        source.blocks(page).await
    })
    .await
}

async fn block_by_height(
    State(state): State<Arc<AppState>>,
    Path(height): Path<u64>,
) -> Result<Json<Value>, ApiError> {
    cached_json(
        state,
        format!("blocks:height:{height}"),
        |source| async move {
            source
                .block_by_height(height)
                .await?
                .ok_or_else(|| ApiError::NotFound(format!("block height {height} not found")))
        },
    )
    .await
}

async fn block_by_hash(
    State(state): State<Arc<AppState>>,
    Path(hash): Path<String>,
) -> Result<Json<Value>, ApiError> {
    cached_json(state, format!("blocks:hash:{hash}"), |source| async move {
        source
            .block_by_hash(&hash)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("block hash {hash} not found")))
    })
    .await
}

async fn account_by_address(
    State(state): State<Arc<AppState>>,
    Path(address): Path<String>,
) -> Result<Json<Value>, ApiError> {
    cached_json(state, format!("accounts:{address}"), |source| async move {
        source
            .account_by_address(&address)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("account {address} not found")))
    })
    .await
}

async fn transaction_by_hash(
    State(state): State<Arc<AppState>>,
    Path(hash): Path<String>,
) -> Result<Json<Value>, ApiError> {
    cached_json(state, format!("transactions:{hash}"), |source| async move {
        source
            .transaction_by_hash(&hash)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("transaction {hash} not found")))
    })
    .await
}

async fn errors(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    cached_json(state, "errors".to_string(), |source| async move {
        source.error_events().await
    })
    .await
}

async fn latest_error(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    cached_json(state, "errors:latest".to_string(), |source| async move {
        source
            .latest_error()
            .await?
            .ok_or_else(|| ApiError::NotFound("no error events found".to_string()))
    })
    .await
}

async fn rollup_batches(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PageQuery>,
) -> Result<Json<Value>, ApiError> {
    let page = query.page_params()?;
    cached_json(
        state,
        format!("rollup:batches:{page}"),
        |source| async move { source.rollup_batches(page).await },
    )
    .await
}

async fn rollup_batch_by_number(
    State(state): State<Arc<AppState>>,
    Path(batch_number): Path<u64>,
) -> Result<Json<Value>, ApiError> {
    cached_json(
        state,
        format!("rollup:batches:{batch_number}"),
        |source| async move {
            source
                .rollup_batch_by_number(batch_number)
                .await?
                .ok_or_else(|| ApiError::NotFound(format!("rollup batch {batch_number} not found")))
        },
    )
    .await
}

async fn search(
    State(state): State<Arc<AppState>>,
    Path(query): Path<String>,
) -> Result<Json<Value>, ApiError> {
    cached_json(state, format!("search:{query}"), |source| async move {
        if let Some(block) = source.block_by_hash(&query).await? {
            return Ok(SearchResult::Block(block));
        }
        if let Ok(height) = query.parse::<u64>() {
            if let Some(block) = source.block_by_height(height).await? {
                return Ok(SearchResult::Block(block));
            }
        }
        if let Some(account) = source.account_by_address(&query).await? {
            return Ok(SearchResult::Account(account));
        }
        if let Some(transaction) = source.transaction_by_hash(&query).await? {
            return Ok(SearchResult::Transaction(transaction));
        }

        Err(ApiError::NotFound(format!("no result for {query}")))
    })
    .await
}

fn read_chain(path: &PathBuf) -> Result<Chain, ApiError> {
    let data = fs::read_to_string(path)
        .map_err(|error| ApiError::ReadChain(format!("{}: {error}", path.display())))?;
    serde_json::from_str(&data)
        .map_err(|error| ApiError::ParseChain(format!("{}: {error}", path.display())))
}

async fn cached_json<T, F, Fut>(
    state: Arc<AppState>,
    key: String,
    load: F,
) -> Result<Json<Value>, ApiError>
where
    T: Serialize,
    F: FnOnce(ChainSource) -> Fut,
    Fut: Future<Output = Result<T, ApiError>>,
{
    if let Some(value) = state.cache_get(&key) {
        return Ok(Json(value));
    }

    let value = serde_json::to_value(load(state.source.clone()).await?)
        .map_err(|error| ApiError::InvalidIndexedData(error.to_string()))?;
    state.cache_set(key, value.clone());
    Ok(Json(value))
}

impl AppState {
    fn cache_get(&self, key: &str) -> Option<Value> {
        if !self.source.cache_enabled() {
            return None;
        }

        let now = Instant::now();
        let mut cache = self.cache.lock().ok()?;
        match cache.entries.get(key) {
            Some(entry) if entry.expires_at > now => Some(entry.value.clone()),
            Some(_) => {
                cache.entries.remove(key);
                None
            }
            None => None,
        }
    }

    fn cache_set(&self, key: String, value: Value) {
        if !self.source.cache_enabled() {
            return;
        }

        if let Ok(mut cache) = self.cache.lock() {
            cache.entries.insert(
                key,
                CacheEntry {
                    expires_at: Instant::now() + POSTGRES_CACHE_TTL,
                    value,
                },
            );
        }
    }
}

impl ChainSource {
    fn cache_enabled(&self) -> bool {
        matches!(self, ChainSource::Postgres { .. })
    }

    async fn chain_summary(&self) -> Result<ChainSummary, ApiError> {
        match self {
            ChainSource::Json { chain_file } => {
                let chain = read_chain(chain_file)?;
                Ok(json_chain_summary(&chain))
            }
            ChainSource::Postgres { pool } => postgres_chain_summary(pool).await,
        }
    }

    async fn blocks(&self, page: PageParams) -> Result<Page<BlockSummary>, ApiError> {
        match self {
            ChainSource::Json { chain_file } => {
                let chain = read_chain(chain_file)?;
                let mut blocks = chain.blocks.iter().map(block_summary).collect::<Vec<_>>();
                blocks.reverse();
                let total = blocks.len();
                let items = blocks
                    .into_iter()
                    .skip(page.offset)
                    .take(page.limit)
                    .collect();
                Ok(page_from_items(items, total, page))
            }
            ChainSource::Postgres { pool } => {
                let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blocks")
                    .fetch_one(pool)
                    .await
                    .map_err(database_error)?;
                let rows = sqlx::query(
                    r#"
                    SELECT height, parent_hash, state_root, block_hash, transaction_count
                    FROM blocks
                    ORDER BY height DESC
                    LIMIT $1 OFFSET $2
                    "#,
                )
                .bind(to_i64_from_usize(page.limit, "page limit")?)
                .bind(to_i64_from_usize(page.offset, "page offset")?)
                .fetch_all(pool)
                .await
                .map_err(database_error)?;

                let items = rows
                    .into_iter()
                    .map(block_summary_from_row)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(page_from_items(
                    items,
                    usize_from_i64(total, "block count")?,
                    page,
                ))
            }
        }
    }

    async fn block_by_height(&self, height: u64) -> Result<Option<BlockView>, ApiError> {
        match self {
            ChainSource::Json { chain_file } => {
                let chain = read_chain(chain_file)?;
                Ok(find_block_by_height(&chain, height).map(block_view))
            }
            ChainSource::Postgres { pool } => {
                let row = sqlx::query("SELECT raw_block FROM blocks WHERE height = $1")
                    .bind(to_i64(height, "block height")?)
                    .fetch_optional(pool)
                    .await
                    .map_err(database_error)?;

                row.map(|row| {
                    parse_json_value::<Block>(row.get("raw_block"), "block")
                        .map(|block| block_view(&block))
                })
                .transpose()
            }
        }
    }

    async fn block_by_hash(&self, hash: &str) -> Result<Option<BlockView>, ApiError> {
        match self {
            ChainSource::Json { chain_file } => {
                let chain = read_chain(chain_file)?;
                Ok(find_block_by_hash(&chain, hash).map(block_view))
            }
            ChainSource::Postgres { pool } => {
                let row = sqlx::query("SELECT raw_block FROM blocks WHERE block_hash = $1")
                    .bind(hash)
                    .fetch_optional(pool)
                    .await
                    .map_err(database_error)?;

                row.map(|row| {
                    parse_json_value::<Block>(row.get("raw_block"), "block")
                        .map(|block| block_view(&block))
                })
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

    async fn error_events(&self) -> Result<Vec<ErrorEventView>, ApiError> {
        match self {
            ChainSource::Json { .. } => Ok(Vec::new()),
            ChainSource::Postgres { pool } => {
                let rows = sqlx::query(
                    r#"
                    SELECT
                        id,
                        service,
                        severity,
                        message,
                        context,
                        created_at::text AS created_at,
                        resolved_at::text AS resolved_at
                    FROM error_events
                    ORDER BY created_at DESC
                    LIMIT 50
                    "#,
                )
                .fetch_all(pool)
                .await
                .map_err(database_error)?;

                rows.into_iter().map(error_event_from_row).collect()
            }
        }
    }

    async fn latest_error(&self) -> Result<Option<ErrorEventView>, ApiError> {
        match self {
            ChainSource::Json { .. } => Ok(None),
            ChainSource::Postgres { pool } => {
                let row = sqlx::query(
                    r#"
                    SELECT
                        id,
                        service,
                        severity,
                        message,
                        context,
                        created_at::text AS created_at,
                        resolved_at::text AS resolved_at
                    FROM error_events
                    WHERE resolved_at IS NULL
                    ORDER BY created_at DESC
                    LIMIT 1
                    "#,
                )
                .fetch_optional(pool)
                .await
                .map_err(database_error)?;

                row.map(error_event_from_row).transpose()
            }
        }
    }

    async fn rollup_batches(&self, page: PageParams) -> Result<Page<RollupBatchSummary>, ApiError> {
        match self {
            ChainSource::Json { chain_file } => {
                let chain = read_chain(chain_file)?;
                let batches = json_rollup_batch(&chain)
                    .map(|batch| vec![batch.summary])
                    .unwrap_or_default();
                let total = batches.len();
                let items = batches
                    .into_iter()
                    .skip(page.offset)
                    .take(page.limit)
                    .collect();
                Ok(page_from_items(items, total, page))
            }
            ChainSource::Postgres { pool } => {
                let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rollup_batches")
                    .fetch_one(pool)
                    .await
                    .map_err(database_error)?;
                let rows = sqlx::query(
                    r#"
                    SELECT
                        batch_number,
                        start_block_height,
                        end_block_height,
                        prev_state_root,
                        new_state_root,
                        transaction_root,
                        block_hash_root,
                        data_hash,
                        transaction_count,
                        status
                    FROM rollup_batches
                    ORDER BY batch_number DESC
                    LIMIT $1 OFFSET $2
                    "#,
                )
                .bind(to_i64_from_usize(page.limit, "page limit")?)
                .bind(to_i64_from_usize(page.offset, "page offset")?)
                .fetch_all(pool)
                .await
                .map_err(database_error)?;

                let items = rows
                    .into_iter()
                    .map(rollup_batch_summary_from_row)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(page_from_items(
                    items,
                    usize_from_i64(total, "rollup batch count")?,
                    page,
                ))
            }
        }
    }

    async fn rollup_batch_by_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<RollupBatchView>, ApiError> {
        match self {
            ChainSource::Json { chain_file } => {
                let chain = read_chain(chain_file)?;
                Ok(json_rollup_batch(&chain)
                    .filter(|batch| batch.summary.batch_number == batch_number))
            }
            ChainSource::Postgres { pool } => {
                let row = sqlx::query(
                    r#"
                    SELECT
                        batch_number,
                        start_block_height,
                        end_block_height,
                        prev_state_root,
                        new_state_root,
                        transaction_root,
                        block_hash_root,
                        data_hash,
                        transaction_count,
                        status
                    FROM rollup_batches
                    WHERE batch_number = $1
                    "#,
                )
                .bind(to_i64(batch_number, "rollup batch number")?)
                .fetch_optional(pool)
                .await
                .map_err(database_error)?;
                let Some(row) = row else {
                    return Ok(None);
                };

                let summary = rollup_batch_summary_from_row(row)?;
                let transaction_rows = sqlx::query(
                    r#"
                    SELECT t.hash, t.block_height, t.transaction_index, t.raw_transfer
                    FROM rollup_batch_transactions rbt
                    JOIN transactions t ON t.hash = rbt.transaction_hash
                    WHERE rbt.batch_number = $1
                    ORDER BY rbt.block_height ASC, rbt.transaction_index ASC
                    "#,
                )
                .bind(to_i64(batch_number, "rollup batch number")?)
                .fetch_all(pool)
                .await
                .map_err(database_error)?;
                let transactions = transaction_rows
                    .into_iter()
                    .map(transaction_from_row)
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(Some(RollupBatchView {
                    summary,
                    transactions,
                }))
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

fn block_view(block: &Block) -> BlockView {
    BlockView {
        height: block.height,
        parent_hash: block.parent_hash.clone(),
        transactions: block.transactions.clone(),
        state_root: block.state_root.clone(),
        block_hash: block.block_hash.clone(),
        transaction_views: block
            .transactions
            .iter()
            .enumerate()
            .map(|(transaction_index, transaction)| TransactionView {
                hash: transaction_hash(transaction),
                block_height: Some(block.height),
                transaction_index,
                transfer: transaction.clone(),
            })
            .collect(),
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

fn error_event_from_row(row: sqlx::postgres::PgRow) -> Result<ErrorEventView, ApiError> {
    Ok(ErrorEventView {
        id: row.get("id"),
        service: row.get("service"),
        severity: row.get("severity"),
        message: row.get("message"),
        context: row.get("context"),
        created_at: row.get("created_at"),
        resolved_at: row.get("resolved_at"),
    })
}

fn rollup_batch_summary_from_row(
    row: sqlx::postgres::PgRow,
) -> Result<RollupBatchSummary, ApiError> {
    Ok(RollupBatchSummary {
        batch_number: from_i64(row.get("batch_number"), "rollup batch number")?,
        start_block_height: from_i64(
            row.get("start_block_height"),
            "rollup batch start block height",
        )?,
        end_block_height: from_i64(row.get("end_block_height"), "rollup batch end block height")?,
        prev_state_root: row.get("prev_state_root"),
        new_state_root: row.get("new_state_root"),
        transaction_root: row.get("transaction_root"),
        block_hash_root: row.get("block_hash_root"),
        data_hash: row.get("data_hash"),
        transaction_count: usize_from_i32(row.get("transaction_count"), "transaction count")?,
        status: row.get("status"),
    })
}

fn json_rollup_batch(chain: &Chain) -> Option<RollupBatchView> {
    let end_height = chain.blocks.last()?.height;
    if end_height == 0 {
        return None;
    }

    let commitment = rollup_batch_commitment(chain, 0, 1, end_height).ok()?;
    Some(rollup_batch_view(chain, commitment, "built"))
}

fn rollup_batch_view(
    chain: &Chain,
    commitment: RollupBatchCommitment,
    status: &str,
) -> RollupBatchView {
    let transactions =
        chain
            .blocks
            .iter()
            .filter(|block| {
                block.height >= commitment.start_block_height
                    && block.height <= commitment.end_block_height
            })
            .flat_map(|block| {
                block.transactions.iter().enumerate().map(
                    move |(transaction_index, transaction)| TransactionView {
                        hash: transaction_hash(transaction),
                        block_height: Some(block.height),
                        transaction_index,
                        transfer: transaction.clone(),
                    },
                )
            })
            .collect();

    RollupBatchView {
        summary: RollupBatchSummary {
            batch_number: commitment.batch_number,
            start_block_height: commitment.start_block_height,
            end_block_height: commitment.end_block_height,
            prev_state_root: commitment.prev_state_root,
            new_state_root: commitment.new_state_root,
            transaction_root: commitment.transaction_root,
            block_hash_root: commitment.block_hash_root,
            data_hash: commitment.data_hash,
            transaction_count: commitment.transaction_count,
            status: status.to_string(),
        },
        transactions,
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

fn to_i64_from_usize(value: usize, label: &str) -> Result<i64, ApiError> {
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
            ApiError::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            ApiError::NotFound(message) => (StatusCode::NOT_FOUND, message),
            ApiError::ReadChain(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
            ApiError::ParseChain(message) => (StatusCode::UNPROCESSABLE_ENTITY, message),
            ApiError::Database(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
            ApiError::InvalidIndexedData(message) => (StatusCode::UNPROCESSABLE_ENTITY, message),
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}
