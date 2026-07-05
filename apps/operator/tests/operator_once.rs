#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use sqlx::{postgres::PgPoolOptions, PgPool};
use tempfile::TempDir;
use tokio::{net::TcpListener, process::Command};

const CONTRACT: &str = "0x1111111111111111111111111111111111111111";
const TX_HASH: &str = "0x7777777777777777777777777777777777777777777777777777777777777777";
const FINALIZED_TOPIC: &str = "0xdc34f737c6cca7c2bef391debb7d95fe9db2db57eb450c20b4ad265834a91e24";

#[derive(Clone)]
struct MockState {
    emit_finalized: Arc<AtomicBool>,
}

#[tokio::test]
#[ignore = "requires local Docker DATABASE_URL"]
async fn once_drives_broadcast_then_indexes_finality() -> Result<()> {
    let database_url =
        std::env::var("DATABASE_URL").context("set DATABASE_URL to local Docker Postgres")?;

    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let schema = format!("operator_test_{}_{}", std::process::id(), nonce);
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .context("connect local Docker DATABASE_URL")?;
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await?;
    let cleanup = SchemaCleanup {
        pool: admin.clone(),
        schema: schema.clone(),
    };
    let isolated_url = schema_url(&database_url, &schema);
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&isolated_url)
        .await?;
    create_fixture_schema(&pool).await?;
    insert_queued_batch(&pool).await?;

    let state = MockState {
        emit_finalized: Arc::new(AtomicBool::new(false)),
    };
    let app = Router::new()
        .route("/", post(rpc))
        .route("/rollup/batches", post(no_pending_transactions))
        .route(
            "/rollup/batches/:batch/validity-witness",
            get(validity_witness),
        )
        .with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move { axum::serve(listener, app).await });

    let fixtures = TempDir::new()?;
    let prover = fixtures.path().join("fake-prover");
    let settlement = fixtures.path().join("fake-settlement-client");
    write_executable(
        &prover,
        r#"#!/bin/sh
set -eu
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--evm-artifact-out" ]; then
    printf '{"proof":"fixture"}\n' > "$2"
    exit 0
  fi
  shift
done
echo "missing --evm-artifact-out" >&2
exit 2
"#,
    )?;
    write_executable(
        &settlement,
        &format!("#!/bin/sh\nset -eu\nprintf '%s\\n' '{TX_HASH}'\n"),
    )?;

    run_once(
        &isolated_url,
        address.port(),
        fixtures.path(),
        &prover,
        &settlement,
    )
    .await?;
    let (status, attempts, hash): (String, i32, Option<String>) = sqlx::query_as(
        "SELECT operator_status, operator_attempts, operator_broadcast_tx_hash \
         FROM rollup_batches WHERE batch_number = 0",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(status, "broadcast");
    assert_eq!(attempts, 1);
    assert_eq!(hash.as_deref(), Some(TX_HASH));

    state.emit_finalized.store(true, Ordering::SeqCst);
    run_once(
        &isolated_url,
        address.port(),
        fixtures.path(),
        &prover,
        &settlement,
    )
    .await?;
    let (batch_status, operator_status, finalize_hash): (String, String, Option<String>) =
        sqlx::query_as(
            "SELECT status, operator_status, l1_finalize_tx_hash \
             FROM rollup_batches WHERE batch_number = 0",
        )
        .fetch_one(&pool)
        .await?;
    assert_eq!(batch_status, "finalized");
    assert_eq!(operator_status, "finalized");
    assert_eq!(finalize_hash.as_deref(), Some(TX_HASH));

    server.abort();
    drop(pool);
    cleanup.drop_schema().await?;
    Ok(())
}

struct SchemaCleanup {
    pool: PgPool,
    schema: String,
}

impl SchemaCleanup {
    async fn drop_schema(self) -> Result<()> {
        sqlx::query(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn schema_url(database_url: &str, schema: &str) -> String {
    let separator = if database_url.contains('?') { '&' } else { '?' };
    format!("{database_url}{separator}options=-csearch_path%3D{schema}")
}

async fn create_fixture_schema(pool: &PgPool) -> Result<()> {
    sqlx::raw_sql(
        r#"
        CREATE TABLE rollup_batches (
            batch_number BIGINT PRIMARY KEY,
            prev_state_root TEXT NOT NULL,
            new_state_root TEXT NOT NULL,
            transaction_root TEXT NOT NULL,
            block_hash_root TEXT NOT NULL,
            data_hash TEXT NOT NULL,
            transaction_count INTEGER NOT NULL,
            status TEXT NOT NULL,
            operator_status TEXT NOT NULL DEFAULT 'queued',
            operator_attempts INTEGER NOT NULL DEFAULT 0,
            operator_last_error TEXT,
            operator_proof_started_at TIMESTAMPTZ,
            operator_proof_completed_at TIMESTAMPTZ,
            operator_broadcast_tx_hash TEXT,
            operator_updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            l1_chain_id BIGINT,
            l1_contract_address TEXT,
            l1_submit_tx_hash TEXT,
            l1_submitted_at TIMESTAMPTZ,
            l1_submit_block_number BIGINT,
            l1_submit_gas_used BIGINT,
            l1_submit_receipt JSONB,
            l1_finalize_tx_hash TEXT,
            l1_finalized_at TIMESTAMPTZ,
            l1_finalize_block_number BIGINT,
            l1_finalize_gas_used BIGINT,
            l1_finalize_receipt JSONB
        );
        CREATE TABLE l1_indexer_checkpoints (
            chain_id BIGINT NOT NULL,
            contract_address TEXT NOT NULL,
            next_block BIGINT NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            PRIMARY KEY (chain_id, contract_address)
        );
        CREATE TABLE error_events (
            service TEXT NOT NULL,
            severity TEXT NOT NULL,
            message TEXT NOT NULL,
            context JSONB NOT NULL
        );
        "#,
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_queued_batch(pool: &PgPool) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO rollup_batches (
            batch_number, prev_state_root, new_state_root, transaction_root,
            block_hash_root, data_hash, transaction_count, status
        ) VALUES (0, $1, $2, $3, $4, $5, 1, 'built')
        "#,
    )
    .bind("11".repeat(32))
    .bind("22".repeat(32))
    .bind("33".repeat(32))
    .bind("44".repeat(32))
    .bind("55".repeat(32))
    .execute(pool)
    .await?;
    Ok(())
}

async fn run_once(
    database_url: &str,
    port: u16,
    artifact_dir: &Path,
    prover: &Path,
    settlement: &Path,
) -> Result<()> {
    let base_url = format!("http://127.0.0.1:{port}");
    let output = Command::new(env!("CARGO_BIN_EXE_operator"))
        .arg("--once")
        .arg("--database-url")
        .arg(database_url)
        .arg("--api-url")
        .arg(&base_url)
        .arg("--rpc-url")
        .arg(&base_url)
        .arg("--contract")
        .arg(CONTRACT)
        .arg("--private-key")
        .arg("0x01")
        .arg("--deployment-block")
        .arg("0")
        .arg("--confirmations")
        .arg("0")
        .arg("--artifact-dir")
        .arg(artifact_dir)
        .arg("--validity-prover-bin")
        .arg(prover)
        .arg("--settlement-client-bin")
        .arg(settlement)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;
    if !output.status.success() {
        bail!(
            "operator --once failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn write_executable(path: &Path, contents: &str) -> Result<()> {
    fs::write(path, contents)?;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

async fn no_pending_transactions() -> (StatusCode, &'static str) {
    (StatusCode::BAD_REQUEST, "no pending transactions")
}

async fn validity_witness() -> Json<Value> {
    Json(json!({"fixture": true}))
}

async fn rpc(State(state): State<MockState>, Json(request): Json<Value>) -> Json<Value> {
    let method = request["method"].as_str().unwrap_or_default();
    let finalized = state.emit_finalized.load(Ordering::SeqCst);
    let result = match method {
        "eth_chainId" => json!("0x7a69"),
        "eth_blockNumber" => json!(if finalized { "0x10" } else { "0xf" }),
        "eth_call" => json!(word_u64(0)),
        "eth_getLogs" if finalized => json!([finalized_log()]),
        "eth_getLogs" => json!([]),
        "eth_getTransactionReceipt" => json!({
            "transactionHash": TX_HASH,
            "blockNumber": "0x10",
            "gasUsed": "0x5208",
            "status": "0x1",
            "to": CONTRACT
        }),
        _ => {
            return Json(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "error": {"code": -32601, "message": format!("unsupported {method}")}
            }))
        }
    };
    Json(json!({"jsonrpc": "2.0", "id": 1, "result": result}))
}

fn finalized_log() -> Value {
    let data = [
        "33".repeat(32),
        "44".repeat(32),
        "55".repeat(32),
        format!("{:064x}", 1),
        "66".repeat(32),
    ]
    .concat();
    json!({
        "address": CONTRACT,
        "topics": [
            FINALIZED_TOPIC,
            word_u64(0),
            format!("0x{}", "11".repeat(32)),
            format!("0x{}", "22".repeat(32))
        ],
        "data": format!("0x{data}"),
        "blockNumber": "0x10",
        "transactionHash": TX_HASH,
        "removed": false
    })
}

fn word_u64(value: u64) -> String {
    format!("0x{value:064x}")
}
