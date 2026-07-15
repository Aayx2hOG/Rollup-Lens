use std::{path::PathBuf, process::Stdio, time::Duration};

use anyhow::{bail, Context, Result};
use clap::Parser;
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{
    postgres::{PgListener, PgPoolOptions},
    PgPool, Postgres, Row, Transaction,
};
use tokio::{fs, process::Command, time::MissedTickBehavior};

const BATCH_FINALIZED_TOPIC: &str =
    "0xdc34f737c6cca7c2bef391debb7d95fe9db2db57eb450c20b4ad265834a91e24";
const DEPOSIT_ACCEPTED_TOPIC: &str =
    "0x68ae3cc39a6751f1e465dbc1b9daec09ddcfd3200f67d8b3e1cbf1166fa914d2";
const TRANSACTION_PENDING_CHANNEL: &str = "rollup_transaction_pending";
const WORK_AVAILABLE_CHANNEL: &str = "rollup_work_available";
const TOKEN_HEADER: &str = "x-operator-api-token";

#[derive(Debug, Parser)]
#[command(about = "Automate rollup sequencing, proving, submission, and L1 indexing")]
struct Args {
    /// Run one lifecycle tick and exit without starting the polling/listener loop.
    #[arg(long, env = "OPERATOR_ONCE")]
    once: bool,
    /// Only index L1DepositInbox logs and skip sequencing, proving, and settlement.
    #[arg(long, env = "OPERATOR_INDEX_DEPOSITS_ONLY")]
    index_deposits_only: bool,
    #[arg(long, env = "DATABASE_URL")]
    database_url: Option<String>,
    /// Deployed Postgres URL used when DATABASE_URL is not set.
    #[arg(long, env = "NEON_DATABASE_URL")]
    neon_database_url: Option<String>,
    #[arg(long, env = "DIRECT_DATABASE_URL")]
    direct_database_url: Option<String>,
    #[arg(
        long,
        env = "SETTLEMENT_API_URL",
        default_value = "http://127.0.0.1:3000"
    )]
    api_url: String,
    #[arg(long = "operator-api-token", env = "OPERATOR_API_TOKEN")]
    operator_token: Option<String>,
    #[arg(long, env = "EVM_RPC_URL")]
    rpc_url: Option<String>,
    #[arg(long, env = "ROLLUP_SETTLEMENT_CONTRACT")]
    contract: Option<String>,
    /// Optional L1DepositInbox contract to index into l1_deposits.
    #[arg(long, env = "L1_DEPOSIT_INBOX")]
    l1_deposit_inbox: Option<String>,
    #[arg(long, env = "L1_DEPOSIT_DEPLOYMENT_BLOCK")]
    l1_deposit_deployment_block: Option<String>,
    #[arg(long, env = "L1_DEPOSIT_REORG_LOOKBACK", default_value_t = 12)]
    l1_deposit_reorg_lookback: u64,
    #[arg(long, env = "EVM_PRIVATE_KEY")]
    private_key: Option<String>,
    #[arg(long, env = "EVM_DEPLOYMENT_BLOCK", default_value_t = 0)]
    deployment_block: u64,
    #[arg(long, env = "EVM_CONFIRMATIONS", default_value_t = 6)]
    confirmations: u64,
    #[arg(long, env = "L1_BATCH_REORG_LOOKBACK", default_value_t = 64)]
    l1_batch_reorg_lookback: u64,
    #[arg(long, env = "L1_LOG_CHUNK_BLOCKS", default_value_t = 10)]
    l1_log_chunk_blocks: u64,
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
    operator_token: Option<String>,
    rpc_url: String,
    contract: Option<String>,
    l1_deposit_inbox: Option<String>,
    l1_deposit_deployment_block: Option<u64>,
    l1_deposit_reorg_lookback: u64,
    private_key: Option<String>,
    deployment_block: u64,
    confirmations: u64,
    l1_batch_reorg_lookback: u64,
    l1_log_chunk_blocks: u64,
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
    block_hash: String,
    transaction_hash: String,
    log_index: String,
    removed: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcBlock {
    number: String,
    hash: String,
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
    block_hash: String,
    tx_hash: String,
    log_index: u32,
}

#[derive(Debug)]
struct DepositEvent {
    deposit_id: u64,
    depositor_l1_address: String,
    recipient_l2_address: String,
    amount: u64,
    rolling_commitment: String,
    block_number: u64,
    block_hash: String,
    tx_hash: String,
    log_index: u32,
}

#[derive(Debug)]
struct L1BlockRef {
    number: u64,
    hash: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let database_url = non_empty(args.database_url.clone())
        .or_else(|| non_empty(args.neon_database_url.clone()))
        .context("set DATABASE_URL for local Docker or NEON_DATABASE_URL for deployment")?;
    let direct_database_url = non_empty(args.direct_database_url.clone());
    let rpc_url = non_empty(args.rpc_url.clone()).context("set EVM_RPC_URL or pass --rpc-url")?;
    let operator_token = non_empty(args.operator_token.clone());
    let contract = non_empty(args.contract.clone());
    let l1_deposit_inbox = non_empty(args.l1_deposit_inbox.clone());
    let private_key = non_empty(args.private_key.clone());
    let l1_deposit_deployment_block = parse_optional_u64_env(
        "L1_DEPOSIT_DEPLOYMENT_BLOCK",
        args.l1_deposit_deployment_block.as_deref(),
    )?;

    if let Some(contract) = &contract {
        validate_address(contract)?;
    }
    if let Some(inbox) = &l1_deposit_inbox {
        validate_address(inbox)?;
    }
    if args.index_deposits_only && l1_deposit_inbox.is_none() {
        bail!("--index-deposits-only requires --l1-deposit-inbox or L1_DEPOSIT_INBOX");
    }
    if !args.index_deposits_only {
        if contract.is_none() {
            bail!("set ROLLUP_SETTLEMENT_CONTRACT or pass --contract");
        }
        if private_key.is_none() {
            bail!("set EVM_PRIVATE_KEY or pass --private-key");
        }
    }
    if args.poll_seconds == 0
        || args.batch_limit == 0
        || args.max_attempts < 1
        || args.l1_log_chunk_blocks == 0
    {
        bail!("poll seconds, batch limit, max attempts, and L1 log chunk blocks must be positive");
    }

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .context("failed to connect to Postgres")?;
    let client = Client::new();
    let chain_id = parse_hex_u64(
        &rpc::<String>(&client, &rpc_url, "eth_chainId", json!([])).await?,
        "chain id",
    )?;
    let operator = Operator {
        pool,
        client,
        api_url: args.api_url.trim_end_matches('/').to_string(),
        operator_token,
        rpc_url,
        contract: contract.map(|address| address.to_ascii_lowercase()),
        l1_deposit_inbox: l1_deposit_inbox.map(|address| address.to_ascii_lowercase()),
        l1_deposit_deployment_block: l1_deposit_deployment_block
            .or((args.deployment_block > 0).then_some(args.deployment_block)),
        l1_deposit_reorg_lookback: args.l1_deposit_reorg_lookback,
        private_key,
        deployment_block: args.deployment_block,
        confirmations: args.confirmations,
        l1_batch_reorg_lookback: args.l1_batch_reorg_lookback,
        l1_log_chunk_blocks: args.l1_log_chunk_blocks,
        batch_limit: args.batch_limit,
        max_attempts: args.max_attempts,
        artifact_dir: args.artifact_dir,
        validity_prover_bin: args.validity_prover_bin,
        settlement_client_bin: args.settlement_client_bin,
        cast_bin: args.cast_bin,
        chain_id,
    };
    fs::create_dir_all(&operator.artifact_dir).await?;

    if args.index_deposits_only && args.once {
        operator.index_deposit_events().await?;
        return Ok(());
    }

    if args.once {
        operator.tick().await?;
        return Ok(());
    }

    if args.index_deposits_only {
        println!(
            "deposit indexer started for chain {} inbox {} with {} confirmations",
            operator.chain_id,
            operator
                .l1_deposit_inbox
                .as_deref()
                .unwrap_or("unconfigured"),
            operator.confirmations,
        );
        let mut interval = tokio::time::interval(Duration::from_secs(args.poll_seconds));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    println!("deposit indexer stopped");
                    return Ok(());
                }
                _ = interval.tick() => {
                    if let Err(error) = operator.index_deposit_events().await {
                        eprintln!("deposit indexer tick failed: {error:#}");
                        operator.record_error(&error).await;
                    }
                }
            }
        }
    }

    let listener_database_url = direct_database_url.as_deref().unwrap_or(&database_url);
    let mut listener = PgListener::connect(listener_database_url)
        .await
        .context("failed to create Postgres pub-sub listener")?;
    listener
        .listen(TRANSACTION_PENDING_CHANNEL)
        .await
        .context("failed to subscribe to pending transaction notifications")?;
    listener
        .listen(WORK_AVAILABLE_CHANNEL)
        .await
        .context("failed to subscribe to rollup work notifications")?;

    println!(
        "operator started for chain {} contract {} with {} confirmations; subscribed to {} and {}",
        operator.chain_id,
        operator.settlement_contract()?,
        operator.confirmations,
        TRANSACTION_PENDING_CHANNEL,
        WORK_AVAILABLE_CHANNEL
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
    fn settlement_contract(&self) -> Result<&str> {
        self.contract
            .as_deref()
            .context("settlement contract is not configured")
    }

    fn settlement_private_key(&self) -> Result<&str> {
        self.private_key
            .as_deref()
            .context("EVM private key is not configured")
    }

    async fn tick(&self) -> Result<()> {
        self.index_finalized_events().await?;
        self.index_deposit_events().await?;
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
        let mut request = self
            .client
            .post(format!("{}/rollup/batches", self.api_url))
            .json(&json!({ "limit": self.batch_limit }));
        request = with_token(request, self.operator_token.as_deref());
        let response = request
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
        let Some(row) = row else {
            return Ok(None);
        };
        let batch = ClaimedBatch {
            batch_number: u64::try_from(row.get::<i64, _>("batch_number"))?,
        };
        self.mark_job_running("prove_batch", &batch.batch_number.to_string())
            .await?;
        Ok(Some(batch))
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
        self.mark_job_succeeded("prove_batch", &batch.batch_number.to_string())
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
        let contract = self.settlement_contract()?;
        let private_key = self.settlement_private_key()?;
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
                .arg(contract)
                .arg("--private-key")
                .arg(private_key)
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
        let contract = self.settlement_contract()?;
        let value: String = rpc(
            &self.client,
            &self.rpc_url,
            "eth_call",
            json!([{"to": contract, "data": "0xc0decb36"}, "latest"]),
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
        self.mark_job_failed("prove_batch", &batch_number.to_string(), error)
            .await?;
        Ok(())
    }

    async fn mark_job_running(&self, job_type: &str, subject_id: &str) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE jobs
            SET status = 'running',
                attempts = attempts + 1,
                locked_by = 'operator',
                locked_at = now(),
                last_error = NULL,
                updated_at = now()
            WHERE job_type = $1
              AND subject_id = $2
              AND status IN ('queued', 'retry', 'running')
            "#,
        )
        .bind(job_type)
        .bind(subject_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn mark_job_succeeded(&self, job_type: &str, subject_id: &str) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE jobs
            SET status = 'succeeded',
                locked_by = NULL,
                locked_at = NULL,
                last_error = NULL,
                updated_at = now()
            WHERE job_type = $1 AND subject_id = $2
            "#,
        )
        .bind(job_type)
        .bind(subject_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn mark_job_failed(
        &self,
        job_type: &str,
        subject_id: &str,
        error: &anyhow::Error,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE jobs
            SET status = CASE
                    WHEN attempts >= max_attempts THEN 'failed'
                    ELSE 'retry'
                END,
                locked_by = NULL,
                locked_at = NULL,
                last_error = $3,
                run_after = now() + interval '30 seconds',
                updated_at = now()
            WHERE job_type = $1 AND subject_id = $2
            "#,
        )
        .bind(job_type)
        .bind(subject_id)
        .bind(truncate(&format!("{error:#}"), 4000))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn index_finalized_events(&self) -> Result<()> {
        let contract = self.settlement_contract()?;
        let safe_block = self
            .block_by_tag("safe")
            .await
            .context("failed to read safe L1 block")?;
        let finalized_block = self
            .block_by_tag("finalized")
            .await
            .context("failed to read finalized L1 block")?;
        let safe_head = safe_block.number;
        let finalized_head = finalized_block.number.min(safe_head);
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
        .bind(contract)
        .bind(i64::try_from(self.deployment_block)?)
        .fetch_one(&self.pool)
        .await?;
        let checkpoint = u64::try_from(next)?;
        let mut from = checkpoint
            .saturating_sub(self.l1_batch_reorg_lookback)
            .max(self.deployment_block);
        if from > safe_head {
            self.promote_batch_finalization_events(contract, finalized_head)
                .await?;
            return Ok(());
        }
        self.orphan_reorged_batch_events(contract, from, safe_head)
            .await?;
        while from <= safe_head {
            let to = safe_head.min(from.saturating_add(self.l1_log_chunk_blocks - 1));
            let logs: Vec<RpcLog> = rpc(
                &self.client,
                &self.rpc_url,
                "eth_getLogs",
                json!([{
                    "address": contract,
                    "fromBlock": hex_u64(from),
                    "toBlock": hex_u64(to),
                    "topics": [BATCH_FINALIZED_TOPIC]
                }]),
            )
            .await?;
            for log in logs {
                if log.removed.unwrap_or(false) {
                    self.orphan_removed_batch_log(contract, &log).await?;
                    continue;
                }
                let event = decode_event(&log)?;
                let status = if event.block_number <= finalized_head {
                    "finalized"
                } else {
                    "safe"
                };
                self.apply_finalized_event(event, status).await?;
            }
            sqlx::query(
                "UPDATE l1_indexer_checkpoints SET next_block = $3, updated_at = now() WHERE chain_id = $1 AND contract_address = $2",
            )
            .bind(i64::try_from(self.chain_id)?)
            .bind(contract)
            .bind(i64::try_from(to.saturating_add(1))?)
            .execute(&self.pool)
            .await?;
            from = to.saturating_add(1);
        }
        self.promote_batch_finalization_events(contract, finalized_head)
            .await?;
        Ok(())
    }

    async fn index_deposit_events(&self) -> Result<()> {
        let Some(inbox) = &self.l1_deposit_inbox else {
            return Ok(());
        };
        let head = parse_hex_u64(
            &rpc::<String>(&self.client, &self.rpc_url, "eth_blockNumber", json!([])).await?,
            "head block",
        )?;
        let Some(confirmed_head) = head.checked_sub(self.confirmations) else {
            return Ok(());
        };
        let deployment_block = self.l1_deposit_deployment_block.unwrap_or(confirmed_head);
        if self.l1_deposit_deployment_block.is_none() {
            println!(
                "L1_DEPOSIT_DEPLOYMENT_BLOCK is not set; starting deposit indexer at confirmed head block {deployment_block}"
            );
        }
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
        .bind(inbox)
        .bind(i64::try_from(deployment_block)?)
        .fetch_one(&self.pool)
        .await?;
        let checkpoint = u64::try_from(next)?;
        let mut from = checkpoint
            .saturating_sub(self.l1_deposit_reorg_lookback)
            .max(deployment_block);
        while from <= confirmed_head {
            let to = confirmed_head.min(from.saturating_add(self.l1_log_chunk_blocks - 1));
            let logs: Vec<RpcLog> = rpc(
                &self.client,
                &self.rpc_url,
                "eth_getLogs",
                json!([{
                    "address": inbox,
                    "fromBlock": hex_u64(from),
                    "toBlock": hex_u64(to),
                    "topics": [DEPOSIT_ACCEPTED_TOPIC]
                }]),
            )
            .await?;
            for log in logs {
                if log.removed.unwrap_or(false) {
                    continue;
                }
                let event = decode_deposit_event(&log)?;
                self.apply_deposit_event(inbox, event, head).await?;
            }
            sqlx::query(
                "UPDATE l1_indexer_checkpoints SET next_block = $3, updated_at = now() WHERE chain_id = $1 AND contract_address = $2",
            )
            .bind(i64::try_from(self.chain_id)?)
            .bind(inbox)
            .bind(i64::try_from(to.saturating_add(1))?)
            .execute(&self.pool)
            .await?;
            from = to.saturating_add(1);
        }
        Ok(())
    }

    async fn apply_deposit_event(&self, inbox: &str, event: DepositEvent, head: u64) -> Result<()> {
        let confirmations = head.saturating_sub(event.block_number).saturating_add(1);
        let status = if confirmations >= self.confirmations.max(1) {
            "confirmed"
        } else {
            "observed"
        };
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            r#"
            UPDATE l1_deposits
            SET status = 'orphaned', updated_at = now()
            WHERE chain_id = $1
              AND inbox_address = $2
              AND deposit_id = $3
              AND status <> 'orphaned'
              AND (l1_block_hash <> $4 OR l1_tx_hash <> $5 OR l1_log_index <> $6)
              AND NOT EXISTS (
                  SELECT 1
                  FROM l1_deposits exact
                  WHERE exact.chain_id = $1
                    AND exact.inbox_address = $2
                    AND exact.deposit_id = $3
                    AND exact.l1_block_hash = $4
                    AND exact.l1_tx_hash = $5
                    AND exact.l1_log_index = $6
              )
            "#,
        )
        .bind(i64::try_from(self.chain_id)?)
        .bind(inbox)
        .bind(i64::try_from(event.deposit_id)?)
        .bind(&event.block_hash)
        .bind(&event.tx_hash)
        .bind(i32::try_from(event.log_index)?)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO l1_deposits (
                chain_id,
                inbox_address,
                deposit_id,
                l1_tx_hash,
                l1_block_number,
                l1_block_hash,
                l1_log_index,
                depositor_l1_address,
                recipient_l2_address,
                amount,
                rolling_commitment,
                status,
                confirmations
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ON CONFLICT (chain_id, inbox_address, deposit_id, l1_block_hash, l1_log_index)
            DO UPDATE SET
                confirmations = GREATEST(l1_deposits.confirmations, EXCLUDED.confirmations),
                status = CASE
                    WHEN l1_deposits.status = 'orphaned' THEN 'orphaned'
                    WHEN EXCLUDED.status = 'confirmed' THEN 'confirmed'
                    ELSE l1_deposits.status
                END,
                updated_at = now()
            "#,
        )
        .bind(i64::try_from(self.chain_id)?)
        .bind(inbox)
        .bind(i64::try_from(event.deposit_id)?)
        .bind(&event.tx_hash)
        .bind(i64::try_from(event.block_number)?)
        .bind(&event.block_hash)
        .bind(i32::try_from(event.log_index)?)
        .bind(&event.depositor_l1_address)
        .bind(&event.recipient_l2_address)
        .bind(i64::try_from(event.amount)?)
        .bind(&event.rolling_commitment)
        .bind(status)
        .bind(i32::try_from(confirmations)?)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        println!(
            "indexed L1 deposit #{} for {} at block {} ({})",
            event.deposit_id, event.recipient_l2_address, event.block_number, event.tx_hash
        );
        Ok(())
    }

    async fn apply_finalized_event(&self, event: FinalizedEvent, status: &str) -> Result<()> {
        let contract = self.settlement_contract()?;
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
                != Some(contract)
            || parse_hex_u64(&receipt.block_number, "receipt block")? != event.block_number
        {
            bail!("invalid receipt for finalized batch {}", event.batch_number);
        }
        let gas_used = parse_hex_u64(&receipt.gas_used, "receipt gas")?;
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            r#"
            UPDATE l1_batch_finalization_events
            SET status = 'orphaned', updated_at = now()
            WHERE chain_id = $1
              AND contract_address = $2
              AND batch_number = $3
              AND status NOT IN ('orphaned', 'finalized')
              AND (l1_block_hash <> $4 OR l1_tx_hash <> $5 OR l1_log_index <> $6)
            "#,
        )
        .bind(i64::try_from(self.chain_id)?)
        .bind(contract)
        .bind(i64::try_from(event.batch_number)?)
        .bind(&event.block_hash)
        .bind(&event.tx_hash)
        .bind(i32::try_from(event.log_index)?)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO l1_batch_finalization_events (
                chain_id,
                contract_address,
                batch_number,
                l1_tx_hash,
                l1_block_number,
                l1_block_hash,
                l1_log_index,
                prev_state_root,
                new_state_root,
                transaction_root,
                block_hash_root,
                data_hash,
                transaction_count,
                batch_commitment,
                status,
                l1_gas_used,
                l1_receipt
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
            ON CONFLICT (chain_id, contract_address, batch_number, l1_block_hash, l1_tx_hash, l1_log_index)
            DO UPDATE SET
                status = CASE
                    WHEN l1_batch_finalization_events.status = 'finalized' THEN 'finalized'
                    WHEN EXCLUDED.status = 'finalized' THEN 'finalized'
                    WHEN l1_batch_finalization_events.status = 'safe' THEN 'safe'
                    WHEN EXCLUDED.status = 'safe' THEN 'safe'
                    ELSE l1_batch_finalization_events.status
                END,
                l1_gas_used = EXCLUDED.l1_gas_used,
                l1_receipt = EXCLUDED.l1_receipt,
                updated_at = now()
            "#,
        )
        .bind(i64::try_from(self.chain_id)?)
        .bind(contract)
        .bind(i64::try_from(event.batch_number)?)
        .bind(&event.tx_hash)
        .bind(i64::try_from(event.block_number)?)
        .bind(&event.block_hash)
        .bind(i32::try_from(event.log_index)?)
        .bind(&event.prev_state_root)
        .bind(&event.new_state_root)
        .bind(&event.transaction_root)
        .bind(&event.block_hash_root)
        .bind(&event.data_hash)
        .bind(i32::try_from(event.transaction_count)?)
        .bind(&event.batch_commitment)
        .bind(status)
        .bind(i64::try_from(gas_used)?)
        .bind(receipt_value.clone())
        .execute(&mut *tx)
        .await?;

        self.project_finalized_event(
            &mut tx,
            contract,
            event.batch_number,
            status,
            &event.tx_hash,
            event.block_number,
            &event.block_hash,
            event.log_index,
            gas_used,
            receipt_value,
        )
        .await?;
        tx.commit().await?;
        println!(
            "indexed {status} batch {} at L1 block {} ({}) commitment {}",
            event.batch_number, event.block_number, event.tx_hash, event.batch_commitment
        );
        Ok(())
    }

    async fn block_by_tag(&self, tag: &str) -> Result<L1BlockRef> {
        let block: RpcBlock = rpc(
            &self.client,
            &self.rpc_url,
            "eth_getBlockByNumber",
            json!([tag, false]),
        )
        .await?;
        Ok(L1BlockRef {
            number: parse_hex_u64(&block.number, "block number")?,
            hash: normalize_root(&block.hash),
        })
    }

    async fn block_by_number(&self, number: u64) -> Result<L1BlockRef> {
        let block: RpcBlock = rpc(
            &self.client,
            &self.rpc_url,
            "eth_getBlockByNumber",
            json!([hex_u64(number), false]),
        )
        .await?;
        Ok(L1BlockRef {
            number: parse_hex_u64(&block.number, "block number")?,
            hash: normalize_root(&block.hash),
        })
    }

    async fn orphan_reorged_batch_events(&self, contract: &str, from: u64, to: u64) -> Result<()> {
        let rows = sqlx::query(
            r#"
            SELECT DISTINCT l1_block_number, l1_block_hash
            FROM l1_batch_finalization_events
            WHERE chain_id = $1
              AND contract_address = $2
              AND status NOT IN ('orphaned', 'finalized')
              AND l1_block_number BETWEEN $3 AND $4
            ORDER BY l1_block_number
            "#,
        )
        .bind(i64::try_from(self.chain_id)?)
        .bind(contract)
        .bind(i64::try_from(from)?)
        .bind(i64::try_from(to)?)
        .fetch_all(&self.pool)
        .await?;

        for row in rows {
            let block_number = u64::try_from(row.get::<i64, _>("l1_block_number"))?;
            let stored_hash: String = row.get("l1_block_hash");
            let changed = match self.block_by_number(block_number).await {
                Ok(current) => current.hash != stored_hash,
                Err(_) => true,
            };
            if changed {
                self.orphan_events_at_block(contract, block_number, &stored_hash)
                    .await?;
            }
        }
        Ok(())
    }

    async fn orphan_events_at_block(
        &self,
        contract: &str,
        block_number: u64,
        block_hash: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(
            r#"
            UPDATE l1_batch_finalization_events
            SET status = 'orphaned', updated_at = now()
            WHERE chain_id = $1
              AND contract_address = $2
              AND l1_block_number = $3
              AND l1_block_hash = $4
              AND status NOT IN ('orphaned', 'finalized')
            RETURNING batch_number, l1_tx_hash, l1_block_hash, l1_log_index
            "#,
        )
        .bind(i64::try_from(self.chain_id)?)
        .bind(contract)
        .bind(i64::try_from(block_number)?)
        .bind(block_hash)
        .fetch_all(&mut *tx)
        .await?;
        for row in rows {
            self.clear_orphaned_batch_projection(
                &mut tx,
                contract,
                u64::try_from(row.get::<i64, _>("batch_number"))?,
                &row.get::<String, _>("l1_tx_hash"),
                &row.get::<String, _>("l1_block_hash"),
                u32::try_from(row.get::<i32, _>("l1_log_index"))?,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn orphan_removed_batch_log(&self, contract: &str, log: &RpcLog) -> Result<()> {
        let block_hash = normalize_root(&log.block_hash);
        let tx_hash = log.transaction_hash.to_ascii_lowercase();
        let log_index = u32::try_from(parse_hex_u64(&log.log_index, "removed log index")?)?;
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(
            r#"
            UPDATE l1_batch_finalization_events
            SET status = 'orphaned', updated_at = now()
            WHERE chain_id = $1
              AND contract_address = $2
              AND l1_block_hash = $3
              AND l1_tx_hash = $4
              AND l1_log_index = $5
              AND status NOT IN ('orphaned', 'finalized')
            RETURNING batch_number
            "#,
        )
        .bind(i64::try_from(self.chain_id)?)
        .bind(contract)
        .bind(&block_hash)
        .bind(&tx_hash)
        .bind(i32::try_from(log_index)?)
        .fetch_all(&mut *tx)
        .await?;
        for row in rows {
            self.clear_orphaned_batch_projection(
                &mut tx,
                contract,
                u64::try_from(row.get::<i64, _>("batch_number"))?,
                &tx_hash,
                &block_hash,
                log_index,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn promote_batch_finalization_events(
        &self,
        contract: &str,
        finalized_head: u64,
    ) -> Result<()> {
        let rows = sqlx::query(
            r#"
            SELECT batch_number, l1_tx_hash, l1_block_number, l1_block_hash, l1_log_index,
                   l1_gas_used, l1_receipt
            FROM l1_batch_finalization_events
            WHERE chain_id = $1
              AND contract_address = $2
              AND status IN ('observed', 'safe')
              AND l1_block_number <= $3
            ORDER BY l1_block_number, l1_log_index
            "#,
        )
        .bind(i64::try_from(self.chain_id)?)
        .bind(contract)
        .bind(i64::try_from(finalized_head)?)
        .fetch_all(&self.pool)
        .await?;

        for row in rows {
            let block_number = u64::try_from(row.get::<i64, _>("l1_block_number"))?;
            let block_hash: String = row.get("l1_block_hash");
            let changed = match self.block_by_number(block_number).await {
                Ok(current) => current.hash != block_hash,
                Err(_) => true,
            };
            if changed {
                self.orphan_events_at_block(contract, block_number, &block_hash)
                    .await?;
                continue;
            }

            let batch_number = u64::try_from(row.get::<i64, _>("batch_number"))?;
            let tx_hash: String = row.get("l1_tx_hash");
            let log_index = u32::try_from(row.get::<i32, _>("l1_log_index"))?;
            let gas_used = u64::try_from(
                row.get::<Option<i64>, _>("l1_gas_used")
                    .context("finalization event lacks receipt gas")?,
            )?;
            let receipt: Value = row
                .get::<Option<Value>, _>("l1_receipt")
                .context("finalization event lacks receipt")?;

            let mut tx = self.pool.begin().await?;
            sqlx::query(
                r#"
                UPDATE l1_batch_finalization_events
                SET status = 'finalized', updated_at = now()
                WHERE chain_id = $1
                  AND contract_address = $2
                  AND batch_number = $3
                  AND l1_block_hash = $4
                  AND l1_tx_hash = $5
                  AND l1_log_index = $6
                  AND status IN ('observed', 'safe')
                "#,
            )
            .bind(i64::try_from(self.chain_id)?)
            .bind(contract)
            .bind(i64::try_from(batch_number)?)
            .bind(&block_hash)
            .bind(&tx_hash)
            .bind(i32::try_from(log_index)?)
            .execute(&mut *tx)
            .await?;
            self.project_finalized_event(
                &mut tx,
                contract,
                batch_number,
                "finalized",
                &tx_hash,
                block_number,
                &block_hash,
                log_index,
                gas_used,
                receipt,
            )
            .await?;
            tx.commit().await?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn project_finalized_event(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        contract: &str,
        batch_number: u64,
        status: &str,
        tx_hash: &str,
        block_number: u64,
        block_hash: &str,
        log_index: u32,
        gas_used: u64,
        receipt: Value,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE rollup_batches SET
                status = CASE
                    WHEN $8 = 'finalized' THEN 'finalized'
                    WHEN status = 'finalized' THEN status
                    ELSE 'submitted'
                END,
                operator_status = CASE
                    WHEN $8 = 'finalized' THEN 'finalized'
                    ELSE 'broadcast'
                END,
                operator_last_error = NULL,
                operator_updated_at = now(),
                l1_chain_id = $2,
                l1_contract_address = $3,
                l1_submit_tx_hash = COALESCE(l1_submit_tx_hash, $4),
                l1_submitted_at = COALESCE(l1_submitted_at, now()),
                l1_submit_block_number = COALESCE(l1_submit_block_number, $5),
                l1_submit_gas_used = COALESCE(l1_submit_gas_used, $9),
                l1_submit_receipt = COALESCE(l1_submit_receipt, $10),
                l1_finalize_tx_hash = $4,
                l1_finalized_at = CASE
                    WHEN $8 = 'finalized' THEN COALESCE(l1_finalized_at, now())
                    ELSE l1_finalized_at
                END,
                l1_finalize_block_number = $5,
                l1_finalize_block_hash = $6,
                l1_finalize_log_index = $7,
                l1_finality_status = $8,
                l1_finalize_gas_used = $9,
                l1_finalize_receipt = $10
            WHERE batch_number = $1
            "#,
        )
        .bind(i64::try_from(batch_number)?)
        .bind(i64::try_from(self.chain_id)?)
        .bind(contract)
        .bind(tx_hash)
        .bind(i64::try_from(block_number)?)
        .bind(block_hash)
        .bind(i32::try_from(log_index)?)
        .bind(status)
        .bind(i64::try_from(gas_used)?)
        .bind(receipt)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    async fn clear_orphaned_batch_projection(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        contract: &str,
        batch_number: u64,
        tx_hash: &str,
        block_hash: &str,
        log_index: u32,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE rollup_batches SET
                status = 'built',
                operator_status = CASE
                    WHEN operator_proof_completed_at IS NOT NULL THEN 'proved'
                    ELSE 'retry'
                END,
                operator_updated_at = now(),
                operator_last_error = 'L1 BatchFinalized event was orphaned by a reorg',
                operator_broadcast_tx_hash = NULL,
                l1_submit_tx_hash = NULL,
                l1_submitted_at = NULL,
                l1_submit_block_number = NULL,
                l1_submit_gas_used = NULL,
                l1_submit_receipt = NULL,
                l1_finalize_tx_hash = NULL,
                l1_finalized_at = NULL,
                l1_finalize_block_number = NULL,
                l1_finalize_block_hash = NULL,
                l1_finalize_log_index = NULL,
                l1_finality_status = 'orphaned',
                l1_finalize_gas_used = NULL,
                l1_finalize_receipt = NULL
            WHERE batch_number = $1
              AND l1_contract_address = $2
              AND l1_finalize_tx_hash = $3
              AND l1_finalize_block_hash = $4
              AND l1_finalize_log_index = $5
              AND COALESCE(l1_finality_status, '') <> 'finalized'
            "#,
        )
        .bind(i64::try_from(batch_number)?)
        .bind(contract)
        .bind(tx_hash)
        .bind(block_hash)
        .bind(i32::try_from(log_index)?)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    async fn record_error(&self, error: &anyhow::Error) {
        let message = truncate(&format!("{error:#}"), 4000);
        let _ = sqlx::query(
            "INSERT INTO error_events (service, severity, message, context) VALUES ('operator', 'error', $1, '{}'::jsonb)",
        )
        .bind(&message)
        .execute(&self.pool)
        .await;
        let _ = sqlx::query(
            r#"
            INSERT INTO system_alerts (service, severity, alert_type, message, context)
            VALUES ('operator', 'error', 'operator_tick_failed', $1, '{}'::jsonb)
            "#,
        )
        .bind(message)
        .execute(&self.pool)
        .await;
    }
}

fn with_token(request: reqwest::RequestBuilder, token: Option<&str>) -> reqwest::RequestBuilder {
    if let Some(token) = token.filter(|token| !token.is_empty()) {
        request.header(TOKEN_HEADER, token)
    } else {
        request
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
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|error| format!("failed to read response body: {error}"));
        bail!(
            "RPC {method} returned HTTP {status}: {}",
            truncate(&body, 1000)
        );
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
        block_hash: normalize_root(&log.block_hash),
        tx_hash: log.transaction_hash.to_ascii_lowercase(),
        log_index: u32::try_from(parse_hex_u64(&log.log_index, "log index")?)?,
    })
}

fn decode_deposit_event(log: &RpcLog) -> Result<DepositEvent> {
    if log.address.to_ascii_lowercase().len() != 42
        || log.topics.len() != 4
        || log.topics[0].to_ascii_lowercase() != DEPOSIT_ACCEPTED_TOPIC
    {
        bail!("malformed DepositAccepted log");
    }
    let words = data_words(&log.data)?;
    if words.len() != 3 {
        bail!("DepositAccepted data must contain three ABI words");
    }
    Ok(DepositEvent {
        deposit_id: word_u64(&log.topics[1], "deposit id")?,
        depositor_l1_address: word_address(&log.topics[2], "depositor")?,
        recipient_l2_address: word_address(&log.topics[3], "L2 recipient")?,
        amount: word_u64(&words[0], "deposit amount")?,
        rolling_commitment: normalize_root(&words[2]),
        block_number: parse_hex_u64(&log.block_number, "deposit block")?,
        block_hash: normalize_root(&log.block_hash),
        tx_hash: log.transaction_hash.to_ascii_lowercase(),
        log_index: u32::try_from(parse_hex_u64(&log.log_index, "deposit log index")?)?,
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

fn word_address(value: &str, label: &str) -> Result<String> {
    let hex = value.strip_prefix("0x").unwrap_or(value);
    if hex.len() != 64 || !hex[..24].bytes().all(|byte| byte == b'0') {
        bail!("{label} is not an ABI-encoded address");
    }
    if !hex[24..].bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{label} contains non-hex characters");
    }
    Ok(format!("0x{}", hex[24..].to_ascii_lowercase()))
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

fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn parse_optional_u64_env(name: &str, value: Option<&str>) -> Result<Option<u64>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    value
        .parse::<u64>()
        .map(Some)
        .with_context(|| format!("{name} must be a non-negative integer"))
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
            block_hash: format!("0x{}", "88".repeat(32)),
            transaction_hash: format!("0x{}", "77".repeat(32)),
            log_index: "0x0".into(),
            removed: Some(false),
        };
        let event = decode_event(&log).unwrap();
        assert_eq!(event.batch_number, 7);
        assert_eq!(event.transaction_count, 9);
        assert_eq!(event.block_number, 10);
        assert_eq!(event.data_hash, word("55"));
    }

    #[test]
    fn decodes_l1_deposit_event() {
        let topic_address = |hex: &str| format!("0x{}{}", "0".repeat(24), hex);
        let depositor = "1111111111111111111111111111111111111111";
        let recipient = "2222222222222222222222222222222222222222";
        let amount = format!("0x{}2a", "0".repeat(62));
        let rolling = format!("0x{}", "33".repeat(32));
        let log = RpcLog {
            address: format!("0x{}", "ab".repeat(20)),
            topics: vec![
                DEPOSIT_ACCEPTED_TOPIC.into(),
                format!("0x{}05", "0".repeat(62)),
                topic_address(depositor),
                topic_address(recipient),
            ],
            data: format!("0x{}{}{}", &amount[2..], "00".repeat(32), &rolling[2..]),
            block_number: "0xb".into(),
            block_hash: format!("0x{}", "44".repeat(32)),
            transaction_hash: format!("0x{}", "55".repeat(32)),
            log_index: "0x7".into(),
            removed: Some(false),
        };

        let event = decode_deposit_event(&log).unwrap();
        assert_eq!(event.deposit_id, 5);
        assert_eq!(event.depositor_l1_address, format!("0x{depositor}"));
        assert_eq!(event.recipient_l2_address, format!("0x{recipient}"));
        assert_eq!(event.amount, 42);
        assert_eq!(event.rolling_commitment, rolling);
        assert_eq!(event.block_number, 11);
        assert_eq!(event.log_index, 7);
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

    #[test]
    fn normalizes_empty_env_values_to_absent() {
        assert_eq!(non_empty(None), None);
        assert_eq!(non_empty(Some("   ".into())), None);
        assert_eq!(non_empty(Some("  value  ".into())), Some("value".into()));
    }

    #[test]
    fn parses_optional_u64_env_after_empty_normalization() {
        assert_eq!(parse_optional_u64_env("TEST_BLOCK", None).unwrap(), None);
        assert_eq!(
            parse_optional_u64_env("TEST_BLOCK", Some("  ")).unwrap(),
            None
        );
        assert_eq!(
            parse_optional_u64_env("TEST_BLOCK", Some("42")).unwrap(),
            Some(42)
        );
        assert!(parse_optional_u64_env("TEST_BLOCK", Some("-1")).is_err());
        assert!(parse_optional_u64_env("TEST_BLOCK", Some("latest")).is_err());
    }

    #[test]
    fn sequencing_request_includes_operator_token_when_configured() {
        let request = with_token(
            reqwest::Client::new().post("http://127.0.0.1:3000/rollup/batches"),
            Some("operator-secret"),
        )
        .build()
        .unwrap();

        assert_eq!(
            request.headers().get(TOKEN_HEADER).unwrap(),
            "operator-secret"
        );
    }
}
