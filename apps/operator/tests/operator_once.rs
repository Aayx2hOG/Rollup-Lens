#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Output, Stdio},
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
const BLOCK_HASH: &str = "0x8888888888888888888888888888888888888888888888888888888888888888";
const FINALIZED_TOPIC: &str = "0xdc34f737c6cca7c2bef391debb7d95fe9db2db57eb450c20b4ad265834a91e24";

#[derive(Clone)]
struct MockState {
    emit_finalized: Arc<AtomicBool>,
    invalid_finalized: Arc<AtomicBool>,
    finalized_tag_at_event: Arc<AtomicBool>,
}

#[tokio::test]
#[ignore = "requires local Docker DATABASE_URL"]
async fn once_drives_broadcast_then_indexes_finality() -> Result<()> {
    let TestDatabase {
        url: isolated_url,
        pool,
        cleanup,
    } = test_database().await?;
    insert_queued_batch(&pool).await?;

    let state = MockState {
        emit_finalized: Arc::new(AtomicBool::new(false)),
        invalid_finalized: Arc::new(AtomicBool::new(false)),
        finalized_tag_at_event: Arc::new(AtomicBool::new(true)),
    };
    let (port, server) = start_mock_server(state.clone()).await?;

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

    run_once(&isolated_url, port, fixtures.path(), &prover, &settlement).await?;
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
    run_once(&isolated_url, port, fixtures.path(), &prover, &settlement).await?;
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

#[tokio::test]
#[ignore = "requires local Docker DATABASE_URL"]
async fn safe_event_does_not_finalize_rollup_batch() -> Result<()> {
    let TestDatabase { url, pool, cleanup } = test_database().await?;
    insert_queued_batch(&pool).await?;
    sqlx::query(
        "UPDATE rollup_batches SET operator_status = 'broadcast', \
         operator_broadcast_tx_hash = $1 WHERE batch_number = 0",
    )
    .bind(TX_HASH)
    .execute(&pool)
    .await?;
    let state = MockState {
        emit_finalized: Arc::new(AtomicBool::new(true)),
        invalid_finalized: Arc::new(AtomicBool::new(false)),
        finalized_tag_at_event: Arc::new(AtomicBool::new(false)),
    };
    let (port, server) = start_mock_server(state).await?;
    let fixtures = TempDir::new()?;
    let unused = fixtures.path().join("unused");
    write_executable(&unused, "#!/bin/sh\nexit 99\n")?;

    run_once(&url, port, fixtures.path(), &unused, &unused).await?;
    let (batch_status, operator_status, finality_status): (String, String, Option<String>) =
        sqlx::query_as(
            "SELECT status, operator_status, l1_finality_status \
             FROM rollup_batches WHERE batch_number = 0",
        )
        .fetch_one(&pool)
        .await?;
    assert_eq!(batch_status, "submitted");
    assert_eq!(operator_status, "broadcast");
    assert_eq!(finality_status.as_deref(), Some("safe"));

    let event_status: String = sqlx::query_scalar(
        "SELECT status FROM l1_batch_finalization_events WHERE batch_number = 0",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(event_status, "safe");

    server.abort();
    drop(pool);
    cleanup.drop_schema().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local Docker DATABASE_URL"]
async fn prover_failure_retries_only_until_max_attempts() -> Result<()> {
    let TestDatabase { url, pool, cleanup } = test_database().await?;
    insert_queued_batch(&pool).await?;
    let state = MockState {
        emit_finalized: Arc::new(AtomicBool::new(false)),
        invalid_finalized: Arc::new(AtomicBool::new(false)),
        finalized_tag_at_event: Arc::new(AtomicBool::new(true)),
    };
    let (port, server) = start_mock_server(state).await?;
    let fixtures = TempDir::new()?;
    let prover = fixtures.path().join("failing-prover");
    let settlement = fixtures.path().join("unused-settlement-client");
    write_executable(
        &prover,
        "#!/bin/sh\nprintf '%s\\n' 'fixture prover failure' >&2\nexit 17\n",
    )?;
    write_executable(&settlement, "#!/bin/sh\nexit 99\n")?;

    for expected_attempt in 1..=3 {
        let output = run_once_output(&url, port, fixtures.path(), &prover, &settlement).await?;
        assert!(
            !output.status.success(),
            "prover failure must fail operator attempt {expected_attempt}"
        );
        let (status, attempts, error): (String, i32, Option<String>) = sqlx::query_as(
            "SELECT operator_status, operator_attempts, operator_last_error \
             FROM rollup_batches WHERE batch_number = 0",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(status, "retry");
        assert_eq!(attempts, expected_attempt);
        assert!(
            error
                .as_deref()
                .is_some_and(|message| message.contains("fixture prover failure")),
            "prover stderr must be persisted"
        );
    }

    let exhausted = run_once_output(&url, port, fixtures.path(), &prover, &settlement).await?;
    assert!(
        exhausted.status.success(),
        "operator must leave an exhausted batch unclaimed: {}",
        String::from_utf8_lossy(&exhausted.stderr)
    );
    let (status, attempts, broadcast_hash): (String, i32, Option<String>) = sqlx::query_as(
        "SELECT operator_status, operator_attempts, operator_broadcast_tx_hash \
         FROM rollup_batches WHERE batch_number = 0",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(status, "retry");
    assert_eq!(attempts, 3);
    assert!(broadcast_hash.is_none());

    server.abort();
    drop(pool);
    cleanup.drop_schema().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires local Docker DATABASE_URL"]
async fn invalid_finalized_event_never_updates_batch() -> Result<()> {
    let TestDatabase { url, pool, cleanup } = test_database().await?;
    insert_queued_batch(&pool).await?;
    sqlx::query(
        "UPDATE rollup_batches SET operator_status = 'broadcast', \
         operator_broadcast_tx_hash = $1 WHERE batch_number = 0",
    )
    .bind(TX_HASH)
    .execute(&pool)
    .await?;
    let state = MockState {
        emit_finalized: Arc::new(AtomicBool::new(true)),
        invalid_finalized: Arc::new(AtomicBool::new(true)),
        finalized_tag_at_event: Arc::new(AtomicBool::new(true)),
    };
    let (port, server) = start_mock_server(state).await?;
    let fixtures = TempDir::new()?;
    let unused = fixtures.path().join("unused");
    write_executable(&unused, "#!/bin/sh\nexit 99\n")?;

    let output = run_once_output(&url, port, fixtures.path(), &unused, &unused).await?;
    assert!(
        !output.status.success(),
        "invalid finalized roots must fail the operator tick"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not match the local commitment"));
    let (batch_status, operator_status, finalize_hash, chain_id): (
        String,
        String,
        Option<String>,
        Option<i64>,
    ) = sqlx::query_as(
        "SELECT status, operator_status, l1_finalize_tx_hash, l1_chain_id \
         FROM rollup_batches WHERE batch_number = 0",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(batch_status, "built");
    assert_eq!(operator_status, "broadcast");
    assert!(finalize_hash.is_none());
    assert!(chain_id.is_none());

    server.abort();
    drop(pool);
    cleanup.drop_schema().await?;
    Ok(())
}

struct TestDatabase {
    url: String,
    pool: PgPool,
    cleanup: SchemaCleanup,
}

async fn test_database() -> Result<TestDatabase> {
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
        pool: admin,
        schema: schema.clone(),
    };
    let url = schema_url(&database_url, &schema);
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await?;
    create_fixture_schema(&pool).await?;
    Ok(TestDatabase { url, pool, cleanup })
}

async fn start_mock_server(state: MockState) -> Result<(u16, tokio::task::JoinHandle<()>)> {
    let app = Router::new()
        .route("/", post(rpc))
        .route("/rollup/batches", post(no_pending_transactions))
        .route(
            "/rollup/batches/:batch/validity-witness",
            get(validity_witness),
        )
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("mock server must remain available")
    });
    Ok((port, server))
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
            l1_finalize_block_hash TEXT,
            l1_finalize_log_index INTEGER,
            l1_finality_status TEXT,
            l1_finalize_gas_used BIGINT,
            l1_finalize_receipt JSONB
        );
        CREATE TABLE l1_batch_finalization_events (
            id BIGSERIAL PRIMARY KEY,
            chain_id BIGINT NOT NULL,
            contract_address TEXT NOT NULL,
            batch_number BIGINT NOT NULL,
            l1_tx_hash TEXT NOT NULL,
            l1_block_number BIGINT NOT NULL,
            l1_block_hash TEXT NOT NULL,
            l1_log_index INTEGER NOT NULL,
            prev_state_root TEXT NOT NULL,
            new_state_root TEXT NOT NULL,
            transaction_root TEXT NOT NULL,
            block_hash_root TEXT NOT NULL,
            data_hash TEXT NOT NULL,
            transaction_count INTEGER NOT NULL,
            batch_commitment TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'observed',
            l1_gas_used BIGINT,
            l1_receipt JSONB,
            observed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            UNIQUE (chain_id, contract_address, batch_number, l1_block_hash, l1_tx_hash, l1_log_index)
        );
        CREATE UNIQUE INDEX l1_batch_finalization_events_canonical_idx
        ON l1_batch_finalization_events(chain_id, contract_address, batch_number)
        WHERE status <> 'orphaned';
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
    let output = run_once_output(database_url, port, artifact_dir, prover, settlement).await?;
    if !output.status.success() {
        bail!(
            "operator --once failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

async fn run_once_output(
    database_url: &str,
    port: u16,
    artifact_dir: &Path,
    prover: &Path,
    settlement: &Path,
) -> Result<Output> {
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
    Ok(output)
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
    let invalid_finalized = state.invalid_finalized.load(Ordering::SeqCst);
    let finalized_tag_at_event = state.finalized_tag_at_event.load(Ordering::SeqCst);
    let result = match method {
        "eth_chainId" => json!("0x7a69"),
        "eth_blockNumber" => json!(if finalized { "0x10" } else { "0xf" }),
        "eth_call" => json!(word_u64(0)),
        "eth_getBlockByNumber" => {
            let tag = request["params"][0].as_str().unwrap_or_default();
            let number = match tag {
                "safe" => {
                    if finalized {
                        "0x10"
                    } else {
                        "0xf"
                    }
                }
                "finalized" => {
                    if finalized && finalized_tag_at_event {
                        "0x10"
                    } else {
                        "0xf"
                    }
                }
                other => other,
            };
            json!({
                "number": number,
                "hash": if number == "0x10" { BLOCK_HASH } else { "0x9999999999999999999999999999999999999999999999999999999999999999" }
            })
        }
        "eth_getLogs" if finalized => json!([finalized_log(invalid_finalized)]),
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

fn finalized_log(invalid: bool) -> Value {
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
            format!("0x{}", if invalid { "99" } else { "22" }.repeat(32))
        ],
        "data": format!("0x{data}"),
        "blockNumber": "0x10",
        "blockHash": BLOCK_HASH,
        "transactionHash": TX_HASH,
        "logIndex": "0x0",
        "removed": false
    })
}

fn word_u64(value: u64) -> String {
    format!("0x{value:064x}")
}
