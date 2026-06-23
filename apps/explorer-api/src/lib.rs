use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chain_core::{
    apply_transaction_to_state, produce_block_from_indexed_state,
    rollup_batch_commitment_from_blocks, state_root, transaction_hash, Account, Block,
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
    pool: PgPool,
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
struct DbSource {
    pool: PgPool,
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

#[derive(Debug, Deserialize)]
struct SequenceBatchRequest {
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SettlementStatusUpdate {
    status: String,
    tx_hash: String,
    contract_address: String,
    chain_id: u64,
    block_number: u64,
    gas_used: u64,
    receipt: Value,
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
struct TransactionPoolEntryView {
    id: u64,
    hash: String,
    status: String,
    failure_reason: Option<String>,
    included_block_height: Option<u64>,
    included_transaction_index: Option<usize>,
    created_at: String,
    updated_at: String,
    transfer: Transfer,
}

#[derive(Debug, Serialize)]
struct SubmittedTransaction {
    hash: String,
    status: String,
    transfer: Transfer,
}

#[derive(Debug, Serialize)]
struct SequencedBatch {
    block: BlockView,
    rollup_batch: Option<RollupBatchSummary>,
    included_transactions: Vec<TransactionPoolEntryView>,
    failed_transactions: Vec<TransactionPoolEntryView>,
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
    l1_chain_id: Option<u64>,
    l1_contract_address: Option<String>,
    l1_submit_tx_hash: Option<String>,
    l1_submitted_at: Option<String>,
    l1_submit_block_number: Option<u64>,
    l1_submit_gas_used: Option<u64>,
    l1_submit_receipt: Option<Value>,
    l1_finalize_tx_hash: Option<String>,
    l1_finalized_at: Option<String>,
    l1_finalize_block_number: Option<u64>,
    l1_finalize_gas_used: Option<u64>,
    l1_finalize_receipt: Option<Value>,
}

#[derive(Debug, Serialize)]
struct RollupBatchView {
    #[serde(flatten)]
    summary: RollupBatchSummary,
    transactions: Vec<TransactionView>,
}

#[derive(Debug, Serialize)]
struct SettlementCalldata {
    contract_name: String,
    function: String,
    signature: String,
    args: SettlementSubmitBatchArgs,
    ordered_args: Vec<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SettlementSubmitBatchArgs {
    batch_number: u64,
    prev_state_root: String,
    new_state_root: String,
    transaction_root: String,
    block_hash_root: String,
    data_hash: String,
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
    Database(String),
    InvalidIndexedData(String),
    Conflict(String),
}

pub fn app_with_pool(pool: PgPool) -> Router {
    let state = Arc::new(AppState {
        pool,
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
        .route(
            "/rollup/transactions",
            get(transaction_pool).post(submit_transaction),
        )
        .route("/errors", get(errors))
        .route("/errors/latest", get(latest_error))
        .route(
            "/rollup/batches",
            get(rollup_batches).post(sequence_rollup_batch),
        )
        .route("/rollup/batches/:batch_number", get(rollup_batch_by_number))
        .route(
            "/rollup/batches/:batch_number/settlement-calldata",
            get(rollup_batch_settlement_calldata),
        )
        .route(
            "/rollup/batches/:batch_number/settlement",
            post(update_rollup_batch_settlement),
        )
        .route("/search/:query", get(search))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn health(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let latest_error = DbSource {
        pool: state.pool.clone(),
    }
    .latest_error()
    .await?;
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
            "GET /rollup/transactions",
            "POST /rollup/transactions",
            "GET /errors",
            "GET /errors/latest",
            "GET /rollup/batches",
            "POST /rollup/batches",
            "GET /rollup/batches/:batch_number",
            "GET /rollup/batches/:batch_number/settlement-calldata",
            "POST /rollup/batches/:batch_number/settlement",
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

async fn transaction_pool(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PageQuery>,
) -> Result<Json<Value>, ApiError> {
    let page = query.page_params()?;
    cached_json(
        state,
        format!("rollup:transactions:{page}"),
        |source| async move { source.transaction_pool(page).await },
    )
    .await
}

async fn submit_transaction(
    State(state): State<Arc<AppState>>,
    Json(transaction): Json<Transfer>,
) -> Result<Json<Value>, ApiError> {
    let submitted = DbSource {
        pool: state.pool.clone(),
    }
    .submit_transaction(transaction)
    .await?;
    state.cache_clear();
    serde_json::to_value(submitted)
        .map(Json)
        .map_err(|error| ApiError::InvalidIndexedData(error.to_string()))
}

async fn sequence_rollup_batch(
    State(state): State<Arc<AppState>>,
    payload: Option<Json<SequenceBatchRequest>>,
) -> Result<Json<Value>, ApiError> {
    let limit = payload
        .and_then(|Json(payload)| payload.limit)
        .unwrap_or(100);
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

    let sequenced = DbSource {
        pool: state.pool.clone(),
    }
    .sequence_rollup_batch(limit)
    .await?;
    state.cache_clear();
    serde_json::to_value(sequenced)
        .map(Json)
        .map_err(|error| ApiError::InvalidIndexedData(error.to_string()))
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

async fn rollup_batch_settlement_calldata(
    State(state): State<Arc<AppState>>,
    Path(batch_number): Path<u64>,
) -> Result<Json<Value>, ApiError> {
    cached_json(
        state,
        format!("rollup:batches:{batch_number}:settlement-calldata"),
        |source| async move {
            let batch = source
                .rollup_batch_by_number(batch_number)
                .await?
                .ok_or_else(|| {
                    ApiError::NotFound(format!("rollup batch {batch_number} not found"))
                })?;

            settlement_calldata_from_batch(&batch.summary)
        },
    )
    .await
}

async fn update_rollup_batch_settlement(
    State(state): State<Arc<AppState>>,
    Path(batch_number): Path<u64>,
    Json(update): Json<SettlementStatusUpdate>,
) -> Result<Json<Value>, ApiError> {
    let updated = DbSource {
        pool: state.pool.clone(),
    }
    .update_rollup_batch_settlement(batch_number, update)
    .await?;
    state.cache_clear();
    serde_json::to_value(updated)
        .map(Json)
        .map_err(|error| ApiError::InvalidIndexedData(error.to_string()))
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

async fn cached_json<T, F, Fut>(
    state: Arc<AppState>,
    key: String,
    load: F,
) -> Result<Json<Value>, ApiError>
where
    T: Serialize,
    F: FnOnce(DbSource) -> Fut,
    Fut: Future<Output = Result<T, ApiError>>,
{
    if let Some(value) = state.cache_get(&key) {
        return Ok(Json(value));
    }

    let value = serde_json::to_value(
        load(DbSource {
            pool: state.pool.clone(),
        })
        .await?,
    )
    .map_err(|error| ApiError::InvalidIndexedData(error.to_string()))?;
    state.cache_set(key, value.clone());
    Ok(Json(value))
}

impl AppState {
    fn cache_get(&self, key: &str) -> Option<Value> {
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

    fn cache_clear(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.entries.clear();
        }
    }
}

impl DbSource {
    async fn chain_summary(&self) -> Result<ChainSummary, ApiError> {
        postgres_chain_summary(&self.pool).await
    }

    async fn blocks(&self, page: PageParams) -> Result<Page<BlockSummary>, ApiError> {
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blocks")
            .fetch_one(&self.pool)
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
        .fetch_all(&self.pool)
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

    async fn block_by_height(&self, height: u64) -> Result<Option<BlockView>, ApiError> {
        let row = sqlx::query("SELECT raw_block FROM blocks WHERE height = $1")
            .bind(to_i64(height, "block height")?)
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;

        row.map(|row| {
            parse_json_value::<Block>(row.get("raw_block"), "block").map(|block| block_view(&block))
        })
        .transpose()
    }

    async fn block_by_hash(&self, hash: &str) -> Result<Option<BlockView>, ApiError> {
        let row = sqlx::query("SELECT raw_block FROM blocks WHERE block_hash = $1")
            .bind(hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;

        row.map(|row| {
            parse_json_value::<Block>(row.get("raw_block"), "block").map(|block| block_view(&block))
        })
        .transpose()
    }

    async fn account_by_address(&self, address: &str) -> Result<Option<Account>, ApiError> {
        let row = sqlx::query(
            r#"
            SELECT address, balance, nonce
            FROM accounts
            WHERE address = $1
            "#,
        )
        .bind(address)
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;

        row.map(account_from_row).transpose()
    }

    async fn transaction_by_hash(&self, hash: &str) -> Result<Option<TransactionView>, ApiError> {
        let row = sqlx::query(
            r#"
            SELECT hash, block_height, transaction_index, raw_transfer
            FROM transactions
            WHERE hash = $1
            "#,
        )
        .bind(hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;

        row.map(transaction_from_row).transpose()
    }

    async fn transaction_pool(
        &self,
        page: PageParams,
    ) -> Result<Page<TransactionPoolEntryView>, ApiError> {
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transaction_pool")
            .fetch_one(&self.pool)
            .await
            .map_err(database_error)?;
        let rows = sqlx::query(
            r#"
            SELECT
                id,
                hash,
                status,
                failure_reason,
                included_block_height,
                included_transaction_index,
                raw_transfer,
                created_at::text AS created_at,
                updated_at::text AS updated_at
            FROM transaction_pool
            ORDER BY id DESC
            LIMIT $1 OFFSET $2
            "#,
        )
        .bind(to_i64_from_usize(page.limit, "page limit")?)
        .bind(to_i64_from_usize(page.offset, "page offset")?)
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;

        let items = rows
            .into_iter()
            .map(transaction_pool_entry_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(page_from_items(
            items,
            usize_from_i64(total, "transaction pool count")?,
            page,
        ))
    }

    async fn submit_transaction(
        &self,
        transaction: Transfer,
    ) -> Result<SubmittedTransaction, ApiError> {
        let hash = transaction_hash(&transaction);
        let duplicate_indexed: Option<String> =
            sqlx::query_scalar("SELECT hash FROM transactions WHERE hash = $1")
                .bind(&hash)
                .fetch_optional(&self.pool)
                .await
                .map_err(database_error)?;
        if duplicate_indexed.is_some() {
            return Err(ApiError::Conflict(format!(
                "transaction {hash} is already indexed"
            )));
        }

        let duplicate_pool: Option<String> =
            sqlx::query_scalar("SELECT hash FROM transaction_pool WHERE hash = $1")
                .bind(&hash)
                .fetch_optional(&self.pool)
                .await
                .map_err(database_error)?;
        if duplicate_pool.is_some() {
            return Err(ApiError::Conflict(format!(
                "transaction {hash} is already in the pool"
            )));
        }

        let mut accounts = load_accounts(&self.pool).await?;
        for pending in load_pending_pool_transfers(&self.pool).await? {
            apply_transaction_to_state(&mut accounts, &pending)
                .map_err(|error| ApiError::BadRequest(error.to_string()))?;
        }
        apply_transaction_to_state(&mut accounts, &transaction)
            .map_err(|error| ApiError::BadRequest(error.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO transaction_pool (
                hash,
                from_address,
                to_address,
                amount,
                nonce,
                public_key_base64,
                signature_base64,
                status,
                raw_transfer
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', $8)
            "#,
        )
        .bind(&hash)
        .bind(&transaction.from)
        .bind(&transaction.to)
        .bind(to_i64(transaction.amount, "transaction amount")?)
        .bind(to_i64(transaction.nonce, "transaction nonce")?)
        .bind(&transaction.public_key_base64)
        .bind(&transaction.signature_base64)
        .bind(serde_json::to_value(&transaction).map_err(|error| {
            ApiError::InvalidIndexedData(format!("invalid transaction json: {error}"))
        })?)
        .execute(&self.pool)
        .await
        .map_err(database_error)?;

        Ok(SubmittedTransaction {
            hash,
            status: "pending".to_string(),
            transfer: transaction,
        })
    }

    async fn sequence_rollup_batch(&self, limit: usize) -> Result<SequencedBatch, ApiError> {
        let mut tx = self.pool.begin().await.map_err(database_error)?;
        let pending_rows = sqlx::query(
            r#"
            SELECT
                id,
                hash,
                status,
                failure_reason,
                included_block_height,
                included_transaction_index,
                raw_transfer,
                created_at::text AS created_at,
                updated_at::text AS updated_at
            FROM transaction_pool
            WHERE status = 'pending'
            ORDER BY id ASC
            LIMIT $1
            FOR UPDATE SKIP LOCKED
            "#,
        )
        .bind(to_i64_from_usize(limit, "sequencer limit")?)
        .fetch_all(&mut *tx)
        .await
        .map_err(database_error)?;

        if pending_rows.is_empty() {
            return Err(ApiError::BadRequest(
                "no pending transactions to sequence".to_string(),
            ));
        }

        let mut accounts = load_accounts_for_update(&mut tx).await?;
        let latest_block = latest_block_for_update(&mut tx).await?;
        let mut next_accounts = accounts.clone();
        let mut accepted = Vec::new();
        let mut failed_ids = Vec::new();

        for row in pending_rows {
            let entry = transaction_pool_entry_from_row(row)?;
            match apply_transaction_to_state(&mut next_accounts, &entry.transfer) {
                Ok(()) => accepted.push(entry),
                Err(error) => failed_ids.push((entry.id, error.to_string())),
            }
        }

        for (id, reason) in &failed_ids {
            sqlx::query(
                r#"
                UPDATE transaction_pool
                SET status = 'failed', failure_reason = $2, updated_at = now()
                WHERE id = $1
                "#,
            )
            .bind(to_i64(*id, "transaction pool id")?)
            .bind(reason)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        if accepted.is_empty() {
            tx.commit().await.map_err(database_error)?;
            return Err(ApiError::BadRequest(
                "all pending transactions failed validation".to_string(),
            ));
        }

        let transactions = accepted
            .iter()
            .map(|entry| entry.transfer.clone())
            .collect::<Vec<_>>();
        let block = produce_block_from_indexed_state(&mut accounts, &latest_block, transactions)
            .map_err(|error| ApiError::BadRequest(error.to_string()))?;

        insert_block_tx(&mut tx, &block).await?;
        for (transaction_index, transaction) in block.transactions.iter().enumerate() {
            insert_indexed_transaction_tx(
                &mut tx,
                &transaction_hash(transaction),
                block.height,
                transaction_index,
                transaction,
            )
            .await?;
        }
        replace_accounts_tx(&mut tx, &accounts).await?;
        sqlx::query(
            r#"
            INSERT INTO indexer_checkpoints (id, last_indexed_height)
            VALUES ('chain', $1)
            ON CONFLICT (id) DO UPDATE
            SET last_indexed_height = EXCLUDED.last_indexed_height
            "#,
        )
        .bind(to_i64(block.height, "last indexed height")?)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        for (transaction_index, entry) in accepted.iter().enumerate() {
            sqlx::query(
                r#"
                UPDATE transaction_pool
                SET
                    status = 'included',
                    failure_reason = NULL,
                    included_block_height = $2,
                    included_transaction_index = $3,
                    updated_at = now()
                WHERE id = $1
                "#,
            )
            .bind(to_i64(entry.id, "transaction pool id")?)
            .bind(to_i64(block.height, "included block height")?)
            .bind(to_i32_from_usize(
                transaction_index,
                "included transaction index",
            )?)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        let rollup_batch = rebuild_rollup_batch_tx(&mut tx).await?;
        tx.commit().await.map_err(database_error)?;

        let included_transactions = accepted
            .into_iter()
            .enumerate()
            .map(|(transaction_index, mut entry)| {
                entry.status = "included".to_string();
                entry.failure_reason = None;
                entry.included_block_height = Some(block.height);
                entry.included_transaction_index = Some(transaction_index);
                entry
            })
            .collect();
        let failed_transactions = load_failed_pool_entries(&self.pool, failed_ids).await?;

        Ok(SequencedBatch {
            block: block_view(&block),
            rollup_batch,
            included_transactions,
            failed_transactions,
        })
    }

    async fn error_events(&self) -> Result<Vec<ErrorEventView>, ApiError> {
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
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;

        rows.into_iter().map(error_event_from_row).collect()
    }

    async fn latest_error(&self) -> Result<Option<ErrorEventView>, ApiError> {
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
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;

        row.map(error_event_from_row).transpose()
    }

    async fn rollup_batches(&self, page: PageParams) -> Result<Page<RollupBatchSummary>, ApiError> {
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rollup_batches")
            .fetch_one(&self.pool)
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
                status,
                l1_chain_id,
                l1_contract_address,
                l1_submit_tx_hash,
                l1_submitted_at::text AS l1_submitted_at,
                l1_submit_block_number,
                l1_submit_gas_used,
                l1_submit_receipt,
                l1_finalize_tx_hash,
                l1_finalized_at::text AS l1_finalized_at,
                l1_finalize_block_number,
                l1_finalize_gas_used,
                l1_finalize_receipt
            FROM rollup_batches
            ORDER BY batch_number DESC
            LIMIT $1 OFFSET $2
            "#,
        )
        .bind(to_i64_from_usize(page.limit, "page limit")?)
        .bind(to_i64_from_usize(page.offset, "page offset")?)
        .fetch_all(&self.pool)
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

    async fn rollup_batch_by_number(
        &self,
        batch_number: u64,
    ) -> Result<Option<RollupBatchView>, ApiError> {
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
                status,
                l1_chain_id,
                l1_contract_address,
                l1_submit_tx_hash,
                l1_submitted_at::text AS l1_submitted_at,
                l1_submit_block_number,
                l1_submit_gas_used,
                l1_submit_receipt,
                l1_finalize_tx_hash,
                l1_finalized_at::text AS l1_finalized_at,
                l1_finalize_block_number,
                l1_finalize_gas_used,
                l1_finalize_receipt
            FROM rollup_batches
            WHERE batch_number = $1
            "#,
        )
        .bind(to_i64(batch_number, "rollup batch number")?)
        .fetch_optional(&self.pool)
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
        .fetch_all(&self.pool)
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

    async fn update_rollup_batch_settlement(
        &self,
        batch_number: u64,
        update: SettlementStatusUpdate,
    ) -> Result<RollupBatchSummary, ApiError> {
        validate_settlement_status(&update.status)?;
        validate_prefixed_hex(&update.tx_hash, 64, "settlement transaction hash")?;
        validate_prefixed_hex(&update.contract_address, 40, "settlement contract address")?;
        validate_receipt(&update.receipt)?;

        let row = match update.status.as_str() {
            "submitted" => sqlx::query(
                r#"
                    UPDATE rollup_batches
                    SET
                        status = 'submitted',
                        l1_chain_id = $2,
                        l1_contract_address = $3,
                        l1_submit_tx_hash = $4,
                        l1_submitted_at = COALESCE(l1_submitted_at, now()),
                        l1_submit_block_number = $5,
                        l1_submit_gas_used = $6,
                        l1_submit_receipt = $7
                    WHERE batch_number = $1
                    RETURNING
                        batch_number,
                        start_block_height,
                        end_block_height,
                        prev_state_root,
                        new_state_root,
                        transaction_root,
                        block_hash_root,
                        data_hash,
                        transaction_count,
                        status,
                        l1_chain_id,
                        l1_contract_address,
                        l1_submit_tx_hash,
                        l1_submitted_at::text AS l1_submitted_at,
                        l1_submit_block_number,
                        l1_submit_gas_used,
                        l1_submit_receipt,
                        l1_finalize_tx_hash,
                        l1_finalized_at::text AS l1_finalized_at,
                        l1_finalize_block_number,
                        l1_finalize_gas_used,
                        l1_finalize_receipt
                    "#,
            )
            .bind(to_i64(batch_number, "rollup batch number")?)
            .bind(to_i64(update.chain_id, "L1 chain id")?)
            .bind(update.contract_address.to_ascii_lowercase())
            .bind(update.tx_hash.to_ascii_lowercase())
            .bind(to_i64(update.block_number, "L1 submit block number")?)
            .bind(to_i64(update.gas_used, "L1 submit gas used")?)
            .bind(update.receipt.clone())
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?,
            "finalized" => sqlx::query(
                r#"
                    UPDATE rollup_batches
                    SET
                        status = 'finalized',
                        l1_chain_id = COALESCE($2, l1_chain_id),
                        l1_contract_address = COALESCE(l1_contract_address, $3),
                        l1_finalize_tx_hash = $4,
                        l1_finalized_at = COALESCE(l1_finalized_at, now()),
                        l1_finalize_block_number = $5,
                        l1_finalize_gas_used = $6,
                        l1_finalize_receipt = $7
                    WHERE
                        batch_number = $1
                        AND l1_submit_tx_hash IS NOT NULL
                        AND (l1_chain_id IS NULL OR l1_chain_id = $2)
                        AND (
                            l1_contract_address IS NULL
                            OR l1_contract_address = lower($3)
                        )
                    RETURNING
                        batch_number,
                        start_block_height,
                        end_block_height,
                        prev_state_root,
                        new_state_root,
                        transaction_root,
                        block_hash_root,
                        data_hash,
                        transaction_count,
                        status,
                        l1_chain_id,
                        l1_contract_address,
                        l1_submit_tx_hash,
                        l1_submitted_at::text AS l1_submitted_at,
                        l1_submit_block_number,
                        l1_submit_gas_used,
                        l1_submit_receipt,
                        l1_finalize_tx_hash,
                        l1_finalized_at::text AS l1_finalized_at,
                        l1_finalize_block_number,
                        l1_finalize_gas_used,
                        l1_finalize_receipt
                    "#,
            )
            .bind(to_i64(batch_number, "rollup batch number")?)
            .bind(to_i64(update.chain_id, "L1 chain id")?)
            .bind(update.contract_address.to_ascii_lowercase())
            .bind(update.tx_hash.to_ascii_lowercase())
            .bind(to_i64(update.block_number, "L1 finalize block number")?)
            .bind(to_i64(update.gas_used, "L1 finalize gas used")?)
            .bind(update.receipt)
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?,
            _ => unreachable!("settlement status validated"),
        }
        .ok_or_else(|| ApiError::NotFound(format!("rollup batch {batch_number} not found")))?;

        rollup_batch_summary_from_row(row)
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
        sqlx::query_scalar("SELECT COUNT(*) FROM transaction_pool WHERE status = 'pending'")
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

async fn load_accounts(pool: &PgPool) -> Result<BTreeMap<String, Account>, ApiError> {
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

    rows.into_iter()
        .map(account_from_row)
        .map(|result| result.map(|account| (account.address.clone(), account)))
        .collect()
}

async fn load_accounts_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<BTreeMap<String, Account>, ApiError> {
    let rows = sqlx::query(
        r#"
        SELECT address, balance, nonce
        FROM accounts
        ORDER BY address ASC
        FOR UPDATE
        "#,
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(database_error)?;

    rows.into_iter()
        .map(account_from_row)
        .map(|result| result.map(|account| (account.address.clone(), account)))
        .collect()
}

async fn load_pending_pool_transfers(pool: &PgPool) -> Result<Vec<Transfer>, ApiError> {
    let rows = sqlx::query(
        r#"
        SELECT raw_transfer
        FROM transaction_pool
        WHERE status = 'pending'
        ORDER BY id ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(database_error)?;

    rows.into_iter()
        .map(|row| parse_json_value(row.get("raw_transfer"), "transaction pool transfer"))
        .collect()
}

async fn latest_block_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<Block, ApiError> {
    let row = sqlx::query(
        r#"
        SELECT raw_block
        FROM blocks
        ORDER BY height DESC
        LIMIT 1
        FOR UPDATE
        "#,
    )
    .fetch_optional(&mut **tx)
    .await
    .map_err(database_error)?
    .ok_or_else(|| {
        ApiError::BadRequest("cannot sequence before genesis block exists".to_string())
    })?;

    parse_json_value(row.get("raw_block"), "latest block")
}

async fn insert_block_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    block: &Block,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"
        INSERT INTO blocks (
            height, parent_hash, state_root, block_hash, transaction_count, raw_block
        )
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(to_i64(block.height, "block height")?)
    .bind(&block.parent_hash)
    .bind(&block.state_root)
    .bind(&block.block_hash)
    .bind(to_i32_from_usize(
        block.transactions.len(),
        "block transaction count",
    )?)
    .bind(
        serde_json::to_value(block)
            .map_err(|error| ApiError::InvalidIndexedData(error.to_string()))?,
    )
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}

async fn insert_indexed_transaction_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    hash: &str,
    block_height: u64,
    transaction_index: usize,
    transaction: &Transfer,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"
        INSERT INTO transactions (
            hash,
            block_height,
            transaction_index,
            from_address,
            to_address,
            amount,
            nonce,
            public_key_base64,
            signature_base64,
            raw_transfer
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#,
    )
    .bind(hash)
    .bind(to_i64(block_height, "transaction block height")?)
    .bind(to_i32_from_usize(transaction_index, "transaction index")?)
    .bind(&transaction.from)
    .bind(&transaction.to)
    .bind(to_i64(transaction.amount, "transaction amount")?)
    .bind(to_i64(transaction.nonce, "transaction nonce")?)
    .bind(&transaction.public_key_base64)
    .bind(&transaction.signature_base64)
    .bind(
        serde_json::to_value(transaction)
            .map_err(|error| ApiError::InvalidIndexedData(error.to_string()))?,
    )
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}

async fn replace_accounts_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    accounts: &BTreeMap<String, Account>,
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM accounts")
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
    for account in accounts.values() {
        sqlx::query(
            r#"
            INSERT INTO accounts (address, balance, nonce)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(&account.address)
        .bind(to_i64(account.balance, "account balance")?)
        .bind(to_i64(account.nonce, "account nonce")?)
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
    }
    Ok(())
}

async fn rebuild_rollup_batch_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<Option<RollupBatchSummary>, ApiError> {
    sqlx::query("DELETE FROM rollup_batch_transactions")
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
    sqlx::query("DELETE FROM rollup_batches")
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;

    let rows = sqlx::query(
        r#"
        SELECT raw_block
        FROM blocks
        ORDER BY height ASC
        "#,
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(database_error)?;
    let blocks = rows
        .into_iter()
        .map(|row| parse_json_value::<Block>(row.get("raw_block"), "block"))
        .collect::<Result<Vec<_>, _>>()?;
    if blocks.len() <= 1 {
        return Ok(None);
    }

    let previous_block = &blocks[0];
    let post_genesis_blocks = &blocks[1..];
    let commitment = rollup_batch_commitment_from_blocks(0, previous_block, post_genesis_blocks)
        .map_err(|error| ApiError::InvalidIndexedData(error.to_string()))?;

    insert_rollup_batch_tx(tx, &commitment, "built").await?;
    for block in post_genesis_blocks {
        for (transaction_index, transaction) in block.transactions.iter().enumerate() {
            let hash = transaction_hash(transaction);
            sqlx::query(
                r#"
                INSERT INTO rollup_batch_transactions (
                    batch_number, transaction_hash, block_height, transaction_index
                )
                VALUES ($1, $2, $3, $4)
                "#,
            )
            .bind(to_i64(commitment.batch_number, "rollup batch number")?)
            .bind(&hash)
            .bind(to_i64(
                block.height,
                "rollup batch transaction block height",
            )?)
            .bind(to_i32_from_usize(
                transaction_index,
                "rollup batch transaction index",
            )?)
            .execute(&mut **tx)
            .await
            .map_err(database_error)?;
        }
    }

    Ok(Some(RollupBatchSummary {
        batch_number: commitment.batch_number,
        start_block_height: commitment.start_block_height,
        end_block_height: commitment.end_block_height,
        prev_state_root: commitment.prev_state_root,
        new_state_root: commitment.new_state_root,
        transaction_root: commitment.transaction_root,
        block_hash_root: commitment.block_hash_root,
        data_hash: commitment.data_hash,
        transaction_count: commitment.transaction_count,
        status: "built".to_string(),
        l1_chain_id: None,
        l1_contract_address: None,
        l1_submit_tx_hash: None,
        l1_submitted_at: None,
        l1_submit_block_number: None,
        l1_submit_gas_used: None,
        l1_submit_receipt: None,
        l1_finalize_tx_hash: None,
        l1_finalized_at: None,
        l1_finalize_block_number: None,
        l1_finalize_gas_used: None,
        l1_finalize_receipt: None,
    }))
}

async fn insert_rollup_batch_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    commitment: &RollupBatchCommitment,
    status: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"
        INSERT INTO rollup_batches (
            batch_number,
            start_block_height,
            end_block_height,
            prev_state_root,
            new_state_root,
            transaction_root,
            block_hash_root,
            data_hash,
            transaction_count,
            status,
            raw_commitment
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
    )
    .bind(to_i64(commitment.batch_number, "rollup batch number")?)
    .bind(to_i64(
        commitment.start_block_height,
        "rollup batch start block height",
    )?)
    .bind(to_i64(
        commitment.end_block_height,
        "rollup batch end block height",
    )?)
    .bind(&commitment.prev_state_root)
    .bind(&commitment.new_state_root)
    .bind(&commitment.transaction_root)
    .bind(&commitment.block_hash_root)
    .bind(&commitment.data_hash)
    .bind(to_i32_from_usize(
        commitment.transaction_count,
        "rollup batch transaction count",
    )?)
    .bind(status)
    .bind(
        serde_json::to_value(commitment)
            .map_err(|error| ApiError::InvalidIndexedData(error.to_string()))?,
    )
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}

async fn load_failed_pool_entries(
    pool: &PgPool,
    failed_ids: Vec<(u64, String)>,
) -> Result<Vec<TransactionPoolEntryView>, ApiError> {
    let mut entries = Vec::new();
    for (id, _) in failed_ids {
        let row = sqlx::query(
            r#"
            SELECT
                id,
                hash,
                status,
                failure_reason,
                included_block_height,
                included_transaction_index,
                raw_transfer,
                created_at::text AS created_at,
                updated_at::text AS updated_at
            FROM transaction_pool
            WHERE id = $1
            "#,
        )
        .bind(to_i64(id, "transaction pool id")?)
        .fetch_one(pool)
        .await
        .map_err(database_error)?;
        entries.push(transaction_pool_entry_from_row(row)?);
    }
    Ok(entries)
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

fn transaction_pool_entry_from_row(
    row: sqlx::postgres::PgRow,
) -> Result<TransactionPoolEntryView, ApiError> {
    Ok(TransactionPoolEntryView {
        id: from_i64(row.get("id"), "transaction pool id")?,
        hash: row.get("hash"),
        status: row.get("status"),
        failure_reason: row.get("failure_reason"),
        included_block_height: row
            .get::<Option<i64>, _>("included_block_height")
            .map(|height| from_i64(height, "included block height"))
            .transpose()?,
        included_transaction_index: row
            .get::<Option<i32>, _>("included_transaction_index")
            .map(|index| usize_from_i32(index, "included transaction index"))
            .transpose()?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        transfer: parse_json_value(row.get("raw_transfer"), "transaction pool transfer")?,
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
        l1_chain_id: row
            .get::<Option<i64>, _>("l1_chain_id")
            .map(|chain_id| from_i64(chain_id, "L1 chain id"))
            .transpose()?,
        l1_contract_address: row.get("l1_contract_address"),
        l1_submit_tx_hash: row.get("l1_submit_tx_hash"),
        l1_submitted_at: row.get("l1_submitted_at"),
        l1_submit_block_number: row
            .get::<Option<i64>, _>("l1_submit_block_number")
            .map(|block_number| from_i64(block_number, "L1 submit block number"))
            .transpose()?,
        l1_submit_gas_used: row
            .get::<Option<i64>, _>("l1_submit_gas_used")
            .map(|gas_used| from_i64(gas_used, "L1 submit gas used"))
            .transpose()?,
        l1_submit_receipt: row.get("l1_submit_receipt"),
        l1_finalize_tx_hash: row.get("l1_finalize_tx_hash"),
        l1_finalized_at: row.get("l1_finalized_at"),
        l1_finalize_block_number: row
            .get::<Option<i64>, _>("l1_finalize_block_number")
            .map(|block_number| from_i64(block_number, "L1 finalize block number"))
            .transpose()?,
        l1_finalize_gas_used: row
            .get::<Option<i64>, _>("l1_finalize_gas_used")
            .map(|gas_used| from_i64(gas_used, "L1 finalize gas used"))
            .transpose()?,
        l1_finalize_receipt: row.get("l1_finalize_receipt"),
    })
}

fn settlement_calldata_from_batch(
    batch: &RollupBatchSummary,
) -> Result<SettlementCalldata, ApiError> {
    let args = SettlementSubmitBatchArgs {
        batch_number: batch.batch_number,
        prev_state_root: bytes32_hex(&batch.prev_state_root, "previous state root")?,
        new_state_root: bytes32_hex(&batch.new_state_root, "new state root")?,
        transaction_root: bytes32_hex(&batch.transaction_root, "transaction root")?,
        block_hash_root: bytes32_hex(&batch.block_hash_root, "block hash root")?,
        data_hash: bytes32_hex(&batch.data_hash, "data hash")?,
    };

    Ok(SettlementCalldata {
        contract_name: "RollupSettlement".to_string(),
        function: "submitBatch".to_string(),
        signature: "submitBatch(uint256,bytes32,bytes32,bytes32,bytes32,bytes32)".to_string(),
        ordered_args: vec![
            json!(args.batch_number),
            json!(args.prev_state_root),
            json!(args.new_state_root),
            json!(args.transaction_root),
            json!(args.block_hash_root),
            json!(args.data_hash),
        ],
        args,
    })
}

fn bytes32_hex(value: &str, label: &str) -> Result<String, ApiError> {
    let hex = value.strip_prefix("0x").unwrap_or(value);
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::InvalidIndexedData(format!(
            "{label} must be a 32-byte hex string"
        )));
    }

    Ok(format!("0x{}", hex.to_ascii_lowercase()))
}

fn validate_prefixed_hex(value: &str, hex_len: usize, label: &str) -> Result<(), ApiError> {
    let Some(hex) = value.strip_prefix("0x") else {
        return Err(ApiError::BadRequest(format!("{label} must start with 0x")));
    };
    if hex.len() != hex_len || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest(format!(
            "{label} must be {hex_len} hex characters"
        )));
    }
    Ok(())
}

fn validate_settlement_status(status: &str) -> Result<(), ApiError> {
    if matches!(status, "submitted" | "finalized") {
        return Ok(());
    }

    Err(ApiError::BadRequest(
        "settlement status must be submitted or finalized".to_string(),
    ))
}

fn validate_receipt(receipt: &Value) -> Result<(), ApiError> {
    if receipt.as_object().is_some() {
        return Ok(());
    }

    Err(ApiError::BadRequest(
        "settlement receipt must be a JSON object".to_string(),
    ))
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

fn to_i32_from_usize(value: usize, label: &str) -> Result<i32, ApiError> {
    i32::try_from(value)
        .map_err(|_| ApiError::InvalidIndexedData(format!("{label} exceeds INTEGER range")))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settlement_calldata_uses_contract_argument_order_and_bytes32_hex() {
        let batch = RollupBatchSummary {
            batch_number: 7,
            start_block_height: 1,
            end_block_height: 3,
            prev_state_root: "A".repeat(64),
            new_state_root: "b".repeat(64),
            transaction_root: "c".repeat(64),
            block_hash_root: "d".repeat(64),
            data_hash: "e".repeat(64),
            transaction_count: 4,
            status: "built".to_string(),
            l1_chain_id: None,
            l1_contract_address: None,
            l1_submit_tx_hash: None,
            l1_submitted_at: None,
            l1_submit_block_number: None,
            l1_submit_gas_used: None,
            l1_submit_receipt: None,
            l1_finalize_tx_hash: None,
            l1_finalized_at: None,
            l1_finalize_block_number: None,
            l1_finalize_gas_used: None,
            l1_finalize_receipt: None,
        };

        let calldata = settlement_calldata_from_batch(&batch).expect("valid calldata");

        assert_eq!(calldata.contract_name, "RollupSettlement");
        assert_eq!(calldata.function, "submitBatch");
        assert_eq!(
            calldata.signature,
            "submitBatch(uint256,bytes32,bytes32,bytes32,bytes32,bytes32)"
        );
        assert_eq!(calldata.args.batch_number, 7);
        assert_eq!(
            calldata.args.prev_state_root,
            format!("0x{}", "a".repeat(64))
        );
        assert_eq!(
            calldata.ordered_args,
            vec![
                json!(7),
                json!(format!("0x{}", "a".repeat(64))),
                json!(format!("0x{}", "b".repeat(64))),
                json!(format!("0x{}", "c".repeat(64))),
                json!(format!("0x{}", "d".repeat(64))),
                json!(format!("0x{}", "e".repeat(64))),
            ]
        );
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            ApiError::NotFound(message) => (StatusCode::NOT_FOUND, message),
            ApiError::Database(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
            ApiError::InvalidIndexedData(message) => (StatusCode::UNPROCESSABLE_ENTITY, message),
            ApiError::Conflict(message) => (StatusCode::CONFLICT, message),
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}
