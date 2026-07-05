use std::{path::PathBuf, process::Stdio, time::Duration};

use anyhow::{bail, Context, Result};
use clap::Parser;
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{
    postgres::{PgListener, PgPoolOptions},
    PgPool, Row,
};
use tokio::{fs, process::Command, time::MissedTickBehavior};

const BATCH_FINALIZED_TOPIC: &str =
    "0xdc34f737c6cca7c2bef391debb7d95fe9db2db57eb450c20b4ad265834a91e24";
const TRANSACTION_PENDING_CHANNEL: &str = "rollup_transaction_pending";

#[derive(Debug, Parser)]
#[command(about = "Automate rollup sequencing, proving, submission, and L1 indexing")]
struct Args {
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,
    #[arg(long, env = "DIRECT_DATABASE_URL")]
    direct_database_url: Option<String>,
    #[arg(
        long,
        env = "SETTLEMENT_API_URL",
        default_value = "http://127.0.0.1:3000"
    )]
    api_url: String,
    #[arg(long, env = "EVM_RPC_URL")]
    rpc_url: String,
    #[arg(long, env = "ROLLUP_SETTLEMENT_CONTRACT")]
    contract: String,
    #[arg(long, env = "EVM_PRIVATE_KEY")]
    private_key: String,
    #[arg(long, env = "EVM_DEPLOYMENT_BLOCK", default_value_t = 0)]
    deployment_block: u64,
    #[arg(long, env = "EVM_CONFIRMATIONS", default_value_t = 6)]
    confirmations: u64,
    #[arg(long, env = "OPERATOR_POLL_SECONDS", default_value_t = 5)]
    poll_seconds: u64,
    #[arg(long, env = "OPERATOR_BATCH_LIMIT", default_value_t = 100)]
    batch_limit: usize,
    #[arg(long, env = "OPERATOR_MAX_ATTEMPTS", default_value_t = 3)]
    max_attempts: i32,
    #[arg(
        long,
        env = "OPERATOR_ARTIFACT_DIR",
        default_value = "operator-artifacts"
    )]
    artifact_dir: PathBuf,
    #[arg(long, env = "VALIDITY_PROVER_BIN", default_value = "validity-prover")]
    validity_prover_bin: PathBuf,
    #[arg(
        long,
        env = "SETTLEMENT_CLIENT_BIN",
        default_value = "settlement-client"
    )]
    settlement_client_bin: PathBuf,
    #[arg(long, env = "CAST_BIN", default_value = "cast")]
    cast_bin: PathBuf,
}

#[derive(Clone)]
struct Operator {
    pool: PgPool,
    client: Client,
    api_url: String,
    rpc_url: String,
    contract: String,
    private_key: String,
    deployment_block: u64,
    confirmations: u64,
    batch_limit: usize,
    max_attempts: i32,
    artifact_dir: PathBuf,
    validity_prover_bin: PathBuf,
    settlement_client_bin: PathBuf,
    cast_bin: PathBuf,
    chain_id: u64,
}

#[derive(Debug)]
struct ClaimedBatch {
    batch_number: u64,
}

#[derive(Debug, Deserialize)]
struct RpcResponse {
    result: Option<Value>,
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcLog {
    address: String,
    topics: Vec<String>,
    data: String,
    block_number: String,
    transaction_hash: String,
    removed: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Receipt {
    transaction_hash: String,
    block_number: String,
    gas_used: String,
    status: String,
    to: Option<String>,
}

#[derive(Debug, Serialize)]
struct RpcRequest<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: Value,
}

#[derive(Debug)]
struct FinalizedEvent {
    batch_number: u64,
    prev_state_root: String,
    new_state_root: String,
    transaction_root: String,
    block_hash_root: String,
    data_hash: String,
    transaction_count: u64,
    batch_commitment: String,
    block_number: u64,
    tx_hash: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    validate_address(&args.contract)?;
    if args.poll_seconds == 0 || args.batch_limit == 0 || args.max_attempts < 1 {
        bail!("poll seconds, batch limit, and max attempts must be positive");
    }

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&args.database_url)
        .await
        .context("failed to connect to Postgres")?;
    let client = Client::new();
    let chain_id = parse_hex_u64(
        &rpc::<String>(&client, &args.rpc_url, "eth_chainId", json!([])).await?,
        "chain id",
    )?;
    let operator = Operator {
        pool,
        client,
        api_url: args.api_url.trim_end_matches('/').to_string(),
        rpc_url: args.rpc_url,
        contract: args.contract.to_ascii_lowercase(),
        private_key: args.private_key,
        deployment_block: args.deployment_block,
        confirmations: args.confirmations,
        batch_limit: args.batch_limit,
        max_attempts: args.max_attempts,
        artifact_dir: args.artifact_dir,
        validity_prover_bin: args.validity_prover_bin,
        settlement_client_bin: args.settlement_client_bin,
        cast_bin: args.cast_bin,
        chain_id,
    };
    fs::create_dir_all(&operator.artifact_dir).await?;
    let listener_database_url = args
        .direct_database_url
        .as_deref()
        .unwrap_or(&args.database_url);
    let mut listener = PgListener::connect(listener_database_url)
        .await
        .context("failed to create Postgres pub-sub listener")?;
    listener
        .listen(TRANSACTION_PENDING_CHANNEL)
        .await
        .context("failed to subscribe to pending transaction notifications")?;

    println!(
        "operator started for chain {} contract {} with {} confirmations; subscribed to {}",
        operator.chain_id, operator.contract, operator.confirmations, TRANSACTION_PENDING_CHANNEL
    );
    let mut interval = tokio::time::interval(Duration::from_secs(args.poll_seconds));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("operator stopped");
                return Ok(());
            }
            _ = interval.tick() => {
                if let Err(error) = operator.tick().await {
                    eprintln!("operator tick failed: {error:#}");
                    operator.record_error(&error).await;
                }
            }
            notification = listener.recv() => {
                match notification {
                    Ok(notification) => {
                        println!(
                            "received {} notification for transaction {}",
                            notification.channel(),
                            notification.payload()
                        );
                        if let Err(error) = operator.tick().await {
                            eprintln!("operator notification tick failed: {error:#}");
                            operator.record_error(&error).await;
                        }
                    }
                    Err(error) => {
                        // PgListener reconnects when possible. The polling branch
                        // remains the durable recovery path if notifications fail.
                        eprintln!("operator notification listener failed: {error}");
                    }
                }
            }
        }
    }
}

impl Operator {
    async fn tick(&self) -> Result<()> {
        self.index_finalized_events().await?;
        self.sequence_pending().await?;
        if let Some(batch) = self.proved_batch_ready_to_submit().await? {
            if let Err(error) = self.submit_proof(&batch).await {
                self.fail_batch(batch.batch_number, &error).await?;
                return Err(error);
            }
            return Ok(());
        }
        if let Some(batch) = self.claim_batch().await? {
            if let Err(error) = self.prove_and_submit(&batch).await {
                self.fail_batch(batch.batch_number, &error).await?;
                return Err(error);
            }
        }
        Ok(())
    }

    async fn sequence_pending(&self) -> Result<()> {
        let response = self
            .client
            .post(format!("{}/rollup/batches", self.api_url))
            .json(&json!({ "limit": self.batch_limit }))
            .send()
            .await
            .context("failed to request sequencing")?;
        if response.status().is_success() || response.status().as_u16() == 409 {
            return Ok(());
        }
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status.as_u16() == 400
            && (body.contains("no pending transactions")
                || body.contains("all pending transactions failed"))
        {
            return Ok(());
        }
        bail!("sequencing request failed with {status}: {body}")
    }

    async fn claim_batch(&self) -> Result<Option<ClaimedBatch>> {
        let onchain_next = self.onchain_next_batch().await?;
        let row = sqlx::query(
            r#"
            UPDATE rollup_batches
            SET operator_status = 'proving',
                operator_attempts = operator_attempts + 1,
                operator_last_error = NULL,
                operator_proof_started_at = now(),
                operator_updated_at = now()
            WHERE batch_number = (
                SELECT batch_number
                FROM rollup_batches
                WHERE batch_number = $2
                  AND status = 'built'
                  AND operator_attempts < $1
                  AND (
                    operator_status IN ('queued', 'retry')
                    OR (operator_status = 'proving' AND operator_updated_at < now() - interval '2 hours')
                  )
                ORDER BY batch_number
                FOR UPDATE SKIP LOCKED
                LIMIT 1
            )
            RETURNING batch_number
            "#,
        )
        .bind(self.max_attempts)
        .bind(i64::try_from(onchain_next)?)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(ClaimedBatch {
                batch_number: u64::try_from(row.get::<i64, _>("batch_number"))?,
            })
        })
        .transpose()
    }

    async fn prove_and_submit(&self, batch: &ClaimedBatch) -> Result<()> {
        let witness_path = self
            .artifact_dir
            .join(format!("batch-{}-witness.json", batch.batch_number));
        let proof_path = self
            .artifact_dir
            .join(format!("batch-{}-proof.json", batch.batch_number));
        let response = self
            .client
            .get(format!(
                "{}/rollup/batches/{}/validity-witness",
                self.api_url, batch.batch_number
            ))
            .send()
            .await?;
        if !response.status().is_success() {
            bail!("witness request failed with {}", response.status());
        }
        fs::write(&witness_path, response.bytes().await?).await?;

        run_command(
            Command::new(&self.validity_prover_bin)
                .arg("groth16")
                .arg(&witness_path)
                .arg("--evm-artifact-out")
                .arg(&proof_path),
            "validity prover",
        )
        .await?;
        sqlx::query(
            "UPDATE rollup_batches SET operator_status = 'proved', operator_proof_completed_at = now(), operator_updated_at = now() WHERE batch_number = $1 AND operator_status = 'proving'",
        )
        .bind(i64::try_from(batch.batch_number)?)
        .execute(&self.pool)
        .await?;

        self.submit_proof(batch).await
    }

    async fn proved_batch_ready_to_submit(&self) -> Result<Option<ClaimedBatch>> {
        let next = self.onchain_next_batch().await?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM rollup_batches WHERE batch_number = $1 AND status = 'built' AND operator_status = 'proved')",
        )
        .bind(i64::try_from(next)?)
        .fetch_one(&self.pool)
        .await?;
        Ok(exists.then_some(ClaimedBatch { batch_number: next }))
    }

    async fn submit_proof(&self, batch: &ClaimedBatch) -> Result<()> {
        let proof_path = self
            .artifact_dir
            .join(format!("batch-{}-proof.json", batch.batch_number));
        let output = run_command(
            Command::new(&self.settlement_client_bin)
                .arg("--api-url")
                .arg(&self.api_url)
                .arg("--rpc-url")
                .arg(&self.rpc_url)
                .arg("--contract")
                .arg(&self.contract)
                .arg("--private-key")
                .arg(&self.private_key)
                .arg("--cast-bin")
                .arg(&self.cast_bin)
                .arg("--no-persist")
                .arg("submit-batch")
                .arg("--batch-number")
                .arg(batch.batch_number.to_string())
                .arg("--proof")
                .arg(&proof_path),
            "settlement client",
        )
        .await?;
        let tx_hash =
            extract_hash(&output).context("settlement output contained no transaction hash")?;
        sqlx::query(
            "UPDATE rollup_batches SET operator_status = 'broadcast', operator_broadcast_tx_hash = $2, operator_updated_at = now() WHERE batch_number = $1",
        )
        .bind(i64::try_from(batch.batch_number)?)
        .bind(tx_hash)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn onchain_next_batch(&self) -> Result<u64> {
        let value: String = rpc(
            &self.client,
            &self.rpc_url,
            "eth_call",
            json!([{"to": self.contract, "data": "0xc0decb36"}, "latest"]),
        )
        .await
        .context("failed to read settlement nextBatchNumber")?;
        word_u64(&value, "on-chain next batch number")
    }

    async fn fail_batch(&self, batch_number: u64, error: &anyhow::Error) -> Result<()> {
        sqlx::query(
            "UPDATE rollup_batches SET operator_status = 'retry', operator_last_error = $2, operator_updated_at = now() WHERE batch_number = $1",
        )
        .bind(i64::try_from(batch_number)?)
        .bind(truncate(&format!("{error:#}"), 4000))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn index_finalized_events(&self) -> Result<()> {
        let head = parse_hex_u64(
            &rpc::<String>(&self.client, &self.rpc_url, "eth_blockNumber", json!([])).await?,
            "head block",
        )?;
        let Some(confirmed_head) = head.checked_sub(self.confirmations) else {
            return Ok(());
        };
        let next: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO l1_indexer_checkpoints (chain_id, contract_address, next_block)
            VALUES ($1, $2, $3)
            ON CONFLICT (chain_id, contract_address) DO UPDATE
            SET contract_address = EXCLUDED.contract_address
            RETURNING next_block
            "#,
        )
        .bind(i64::try_from(self.chain_id)?)
        .bind(&self.contract)
        .bind(i64::try_from(self.deployment_block)?)
        .fetch_one(&self.pool)
        .await?;
        let mut from = u64::try_from(next)?;
        while from <= confirmed_head {
            let to = confirmed_head.min(from.saturating_add(999));
            let logs: Vec<RpcLog> = rpc(
                &self.client,
                &self.rpc_url,
                "eth_getLogs",
                json!([{
                    "address": self.contract,
                    "fromBlock": hex_u64(from),
                    "toBlock": hex_u64(to),
                    "topics": [BATCH_FINALIZED_TOPIC]
                }]),
            )
            .await?;
            for log in logs {
                if log.removed.unwrap_or(false) {
                    continue;
                }
                let event = decode_event(&log)?;
                self.apply_finalized_event(event).await?;
            }
            sqlx::query(
                "UPDATE l1_indexer_checkpoints SET next_block = $3, updated_at = now() WHERE chain_id = $1 AND contract_address = $2",
            )
            .bind(i64::try_from(self.chain_id)?)
            .bind(&self.contract)
            .bind(i64::try_from(to.saturating_add(1))?)
            .execute(&self.pool)
            .await?;
            from = to.saturating_add(1);
        }
        Ok(())
    }

    async fn apply_finalized_event(&self, event: FinalizedEvent) -> Result<()> {
        let row = sqlx::query(
            r#"
            SELECT prev_state_root, new_state_root, transaction_root, block_hash_root,
                   data_hash, transaction_count
            FROM rollup_batches WHERE batch_number = $1
            "#,
        )
        .bind(i64::try_from(event.batch_number)?)
        .fetch_optional(&self.pool)
        .await?
        .with_context(|| format!("L1 finalized unknown batch {}", event.batch_number))?;
        let expected = [
            row.get::<String, _>("prev_state_root"),
            row.get::<String, _>("new_state_root"),
            row.get::<String, _>("transaction_root"),
            row.get::<String, _>("block_hash_root"),
            row.get::<String, _>("data_hash"),
        ];
        let actual = [
            event.prev_state_root.clone(),
            event.new_state_root.clone(),
            event.transaction_root.clone(),
            event.block_hash_root.clone(),
            event.data_hash.clone(),
        ];
        if expected
            .iter()
            .zip(actual.iter())
            .any(|(a, b)| normalize_root(a) != normalize_root(b))
            || u64::try_from(row.get::<i32, _>("transaction_count"))? != event.transaction_count
        {
            bail!(
                "L1 event for batch {} does not match the local commitment",
                event.batch_number
            );
        }

        let receipt_value: Value = rpc(
            &self.client,
            &self.rpc_url,
            "eth_getTransactionReceipt",
            json!([event.tx_hash]),
        )
        .await?;
        let receipt: Receipt = serde_json::from_value(receipt_value.clone())?;
        if receipt.status != "0x1"
            || receipt.transaction_hash.to_ascii_lowercase() != event.tx_hash
            || receipt
                .to
                .as_deref()
                .map(str::to_ascii_lowercase)
                .as_deref()
                != Some(self.contract.as_str())
            || parse_hex_u64(&receipt.block_number, "receipt block")? != event.block_number
        {
            bail!("invalid receipt for finalized batch {}", event.batch_number);
        }
        let gas_used = parse_hex_u64(&receipt.gas_used, "receipt gas")?;
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            r#"
            UPDATE rollup_batches SET
                status = 'finalized',
                operator_status = 'finalized',
                operator_last_error = NULL,
                operator_updated_at = now(),
                l1_chain_id = $2,
                l1_contract_address = $3,
                l1_submit_tx_hash = $4,
                l1_submitted_at = COALESCE(l1_submitted_at, now()),
                l1_submit_block_number = $5,
                l1_submit_gas_used = $6,
                l1_submit_receipt = $7,
                l1_finalize_tx_hash = $4,
                l1_finalized_at = COALESCE(l1_finalized_at, now()),
                l1_finalize_block_number = $5,
                l1_finalize_gas_used = $6,
                l1_finalize_receipt = $7
            WHERE batch_number = $1
            "#,
        )
        .bind(i64::try_from(event.batch_number)?)
        .bind(i64::try_from(self.chain_id)?)
        .bind(&self.contract)
        .bind(&event.tx_hash)
        .bind(i64::try_from(event.block_number)?)
        .bind(i64::try_from(gas_used)?)
        .bind(receipt_value)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        println!(
            "indexed finalized batch {} at L1 block {} ({}) commitment {}",
            event.batch_number, event.block_number, event.tx_hash, event.batch_commitment
        );
        Ok(())
    }

    async fn record_error(&self, error: &anyhow::Error) {
        let _ = sqlx::query(
            "INSERT INTO error_events (service, severity, message, context) VALUES ('operator', 'error', $1, '{}'::jsonb)",
        )
        .bind(truncate(&format!("{error:#}"), 4000))
        .execute(&self.pool)
        .await;
    }
}

async fn run_command(command: &mut Command, label: &str) -> Result<String> {
    let output = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("failed to start {label}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        bail!(
            "{label} exited with {}: {}",
            output.status,
            truncate(&stderr, 2000)
        );
    }
    if !stdout.trim().is_empty() {
        println!("{stdout}");
    }
    Ok(format!("{stdout}\n{stderr}"))
}

async fn rpc<T: DeserializeOwned>(
    client: &Client,
    url: &str,
    method: &str,
    params: Value,
) -> Result<T> {
    let response = client
        .post(url)
        .json(&RpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method,
            params,
        })
        .send()
        .await
        .with_context(|| format!("RPC {method} failed"))?;
    if !response.status().is_success() {
        bail!("RPC {method} returned HTTP {}", response.status());
    }
    let body: RpcResponse = response.json().await?;
    if let Some(error) = body.error {
        bail!("RPC {method} error {}: {}", error.code, error.message);
    }
    serde_json::from_value(body.result.context("RPC result was null")?)
        .with_context(|| format!("invalid RPC {method} result"))
}

fn decode_event(log: &RpcLog) -> Result<FinalizedEvent> {
    if log.address.to_ascii_lowercase().len() != 42
        || log.topics.len() != 4
        || log.topics[0].to_ascii_lowercase() != BATCH_FINALIZED_TOPIC
    {
        bail!("malformed BatchFinalized log");
    }
    let words = data_words(&log.data)?;
    if words.len() != 5 {
        bail!("BatchFinalized data must contain five ABI words");
    }
    Ok(FinalizedEvent {
        batch_number: word_u64(&log.topics[1], "batch number")?,
        prev_state_root: normalize_root(&log.topics[2]),
        new_state_root: normalize_root(&log.topics[3]),
        transaction_root: words[0].clone(),
        block_hash_root: words[1].clone(),
        data_hash: words[2].clone(),
        transaction_count: word_u64(&words[3], "transaction count")?,
        batch_commitment: words[4].clone(),
        block_number: parse_hex_u64(&log.block_number, "log block")?,
        tx_hash: log.transaction_hash.to_ascii_lowercase(),
    })
}

fn data_words(value: &str) -> Result<Vec<String>> {
    let hex = value.strip_prefix("0x").context("event data lacks 0x")?;
    if hex.len() % 64 != 0 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("event data is not ABI words");
    }
    Ok(hex
        .as_bytes()
        .chunks(64)
        .map(|chunk| format!("0x{}", String::from_utf8_lossy(chunk).to_ascii_lowercase()))
        .collect())
}

fn word_u64(value: &str, label: &str) -> Result<u64> {
    let hex = value.strip_prefix("0x").unwrap_or(value);
    if hex.len() != 64 || !hex[..48].bytes().all(|byte| byte == b'0') {
        bail!("{label} exceeds u64");
    }
    u64::from_str_radix(&hex[48..], 16).with_context(|| format!("invalid {label}"))
}

fn parse_hex_u64(value: &str, label: &str) -> Result<u64> {
    u64::from_str_radix(value.strip_prefix("0x").context("hex value lacks 0x")?, 16)
        .with_context(|| format!("invalid {label}: {value}"))
}

fn normalize_root(value: &str) -> String {
    format!("0x{}", value.trim_start_matches("0x").to_ascii_lowercase())
}

fn hex_u64(value: u64) -> String {
    format!("0x{value:x}")
}

fn validate_address(value: &str) -> Result<()> {
    let hex = value
        .strip_prefix("0x")
        .context("contract address must start with 0x")?;
    if hex.len() != 40 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("contract address must contain 20 hexadecimal bytes");
    }
    Ok(())
}

fn extract_hash(output: &str) -> Option<String> {
    output
        .split(|c: char| !c.is_ascii_hexdigit() && c != 'x')
        .find_map(|part| {
            let hex = part.strip_prefix("0x")?;
            (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| format!("0x{}", hex.to_ascii_lowercase()))
        })
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_validity_finalization_event() {
        let word = |byte: &str| format!("0x{}", byte.repeat(32));
        let count = format!("0x{}09", "0".repeat(62));
        let log = RpcLog {
            address: format!("0x{}", "ab".repeat(20)),
            topics: vec![
                BATCH_FINALIZED_TOPIC.into(),
                format!("0x{}07", "0".repeat(62)),
                word("11"),
                word("22"),
            ],
            data: format!(
                "0x{}{}{}{}{}",
                "33".repeat(32),
                "44".repeat(32),
                "55".repeat(32),
                &count[2..],
                "66".repeat(32)
            ),
            block_number: "0xa".into(),
            transaction_hash: format!("0x{}", "77".repeat(32)),
            removed: Some(false),
        };
        let event = decode_event(&log).unwrap();
        assert_eq!(event.batch_number, 7);
        assert_eq!(event.transaction_count, 9);
        assert_eq!(event.block_number, 10);
        assert_eq!(event.data_hash, word("55"));
    }

    #[test]
    fn extracts_cast_transaction_hash() {
        let hash = format!("0x{}", "A1".repeat(32));
        assert_eq!(
            extract_hash(&format!("transactionHash {hash}\n")),
            Some(hash.to_ascii_lowercase())
        );
    }

    #[test]
    fn rejects_oversized_abi_integer() {
        assert!(word_u64(&format!("0x01{}", "00".repeat(31)), "number").is_err());
    }
}
