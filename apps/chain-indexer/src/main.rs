use std::{
    env, fs,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use chain_core::{rollup_batch_commitment, transaction_hash, validate_chain, Chain};
use clap::Parser;
use sqlx::{postgres::PgPoolOptions, Executor, PgPool};

const INIT_SQL: &[&str] = &[
    include_str!("../../../prisma/migrations/0001_init/migration.sql"),
    include_str!("../../../prisma/migrations/0002_error_events/migration.sql"),
    include_str!("../../../prisma/migrations/0003_rollup_batches/migration.sql"),
];
const PRISMA_ENV_PATH: &str = "prisma/.env";

#[derive(Debug, Parser)]
#[command(name = "chain-indexer")]
#[command(about = "Indexes the local chain JSON file into Prisma/Postgres tables")]
struct Cli {
    #[arg(long, default_value = "chain.json")]
    chain_file: PathBuf,

    #[arg(long)]
    database_url: Option<String>,

    #[arg(long)]
    init_schema: bool,

    #[arg(long)]
    watch: bool,

    #[arg(long, default_value_t = 5)]
    poll_interval_seconds: u64,

    #[arg(long)]
    backup_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.poll_interval_seconds == 0 {
        anyhow::bail!("--poll-interval-seconds must be greater than zero");
    }

    let database_url = database_url(cli.database_url)?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .context("failed to connect to Postgres")?;

    if cli.init_schema {
        init_schema(&pool).await?;
    }

    if cli.watch {
        watch_chain(
            &pool,
            cli.chain_file,
            Duration::from_secs(cli.poll_interval_seconds),
            cli.backup_dir,
        )
        .await?;
    } else {
        let chain_data = match read_chain_data(&cli.chain_file) {
            Ok(chain_data) => chain_data,
            Err(error) => {
                record_error_event(
                    &pool,
                    "error",
                    "failed to read chain file",
                    serde_json::json!({
                        "chain_file": cli.chain_file.display().to_string(),
                        "error": format!("{error:#}"),
                    }),
                )
                .await;
                return Err(error);
            }
        };
        let result = match index_chain_data(&pool, &cli.chain_file, &chain_data).await {
            Ok(result) => result,
            Err(error) => {
                record_error_event(
                    &pool,
                    "error",
                    "index run failed",
                    serde_json::json!({
                        "chain_file": cli.chain_file.display().to_string(),
                        "error": format!("{error:#}"),
                    }),
                )
                .await;
                return Err(error);
            }
        };
        if let Some(backup_dir) = cli.backup_dir {
            if let Err(error) = write_chain_backup(&backup_dir, &chain_data, result) {
                record_error_event(
                    &pool,
                    "warning",
                    "failed to write chain backup",
                    serde_json::json!({
                        "backup_dir": backup_dir.display().to_string(),
                        "error": format!("{error:#}"),
                    }),
                )
                .await;
                return Err(error);
            }
        }
        print_index_result("indexed", result)?;
    }

    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct IndexResult {
    blocks: usize,
    transactions: usize,
    accounts: usize,
    rollup_batches: usize,
    last_indexed_height: Option<u64>,
}

fn database_url(cli_value: Option<String>) -> Result<String> {
    cli_value
        .or_else(|| env::var("DATABASE_URL").ok())
        .or_else(database_url_from_prisma_env)
        .filter(|value| !value.trim().is_empty())
        .context("provide --database-url, set DATABASE_URL, or add DATABASE_URL to prisma/.env")
}

fn database_url_from_prisma_env() -> Option<String> {
    let entries = dotenvy::from_path_iter(PRISMA_ENV_PATH).ok()?;
    for entry in entries.flatten() {
        let (key, value) = entry;
        if key == "DATABASE_URL" {
            return Some(value);
        }
    }
    None
}

fn read_chain_data(path: &PathBuf) -> Result<String> {
    fs::read_to_string(path)
        .with_context(|| format!("failed to read chain file {}", path.display()))
}

fn parse_chain(path: &PathBuf, data: &str) -> Result<Chain> {
    serde_json::from_str(data)
        .with_context(|| format!("failed to parse chain file {}", path.display()))
}

async fn init_schema(pool: &PgPool) -> Result<()> {
    for migration in INIT_SQL {
        for statement in migration
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            pool.execute(statement)
                .await
                .with_context(|| format!("failed to execute schema statement: {statement}"))?;
        }
    }

    Ok(())
}

async fn watch_chain(
    pool: &PgPool,
    chain_file: PathBuf,
    poll_interval: Duration,
    backup_dir: Option<PathBuf>,
) -> Result<()> {
    let mut last_indexed_data: Option<String> = None;
    let mut last_error_signature: Option<String> = None;

    println!(
        "watching {} every {}s",
        chain_file.display(),
        poll_interval.as_secs()
    );

    loop {
        match read_chain_data(&chain_file) {
            Ok(chain_data) if last_indexed_data.as_ref() == Some(&chain_data) => {
                println!("chain file unchanged; skipping index run");
            }
            Ok(chain_data) => match index_chain_data(pool, &chain_file, &chain_data).await {
                Ok(result) => {
                    if let Some(backup_dir) = &backup_dir {
                        if let Err(error) = write_chain_backup(backup_dir, &chain_data, result) {
                            eprintln!("failed to write chain backup: {error:#}");
                            record_error_event(
                                pool,
                                "warning",
                                "failed to write chain backup",
                                serde_json::json!({
                                    "backup_dir": backup_dir.display().to_string(),
                                    "error": format!("{error:#}"),
                                }),
                            )
                            .await;
                        }
                    }
                    print_index_result("indexed", result)?;
                    last_indexed_data = Some(chain_data);
                    last_error_signature = None;
                }
                Err(error) => {
                    eprintln!("index run failed: {error:#}");
                    let error_text = format!("{error:#}");
                    let signature = format!("index:{error_text}");
                    if last_error_signature.as_ref() != Some(&signature) {
                        record_error_event(
                            pool,
                            "error",
                            "index run failed",
                            serde_json::json!({
                                "chain_file": chain_file.display().to_string(),
                                "error": error_text,
                            }),
                        )
                        .await;
                        last_error_signature = Some(signature);
                    }
                }
            },
            Err(error) => {
                eprintln!("failed to read chain file: {error:#}");
                let error_text = format!("{error:#}");
                let signature = format!("read:{error_text}");
                if last_error_signature.as_ref() != Some(&signature) {
                    record_error_event(
                        pool,
                        "error",
                        "failed to read chain file",
                        serde_json::json!({
                            "chain_file": chain_file.display().to_string(),
                            "error": error_text,
                        }),
                    )
                    .await;
                    last_error_signature = Some(signature);
                }
            }
        }

        tokio::time::sleep(poll_interval).await;
    }
}

fn write_chain_backup(
    backup_dir: &PathBuf,
    chain_data: &str,
    result: IndexResult,
) -> Result<PathBuf> {
    fs::create_dir_all(backup_dir)
        .with_context(|| format!("failed to create backup dir {}", backup_dir.display()))?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_secs();
    let height = result
        .last_indexed_height
        .map(|height| height.to_string())
        .unwrap_or_else(|| "none".to_string());
    let backup_path = backup_dir.join(format!("chain-{timestamp}-height-{height}.json"));

    fs::write(&backup_path, chain_data)
        .with_context(|| format!("failed to write backup {}", backup_path.display()))?;
    println!("wrote chain backup {}", backup_path.display());

    Ok(backup_path)
}

async fn record_error_event(
    pool: &PgPool,
    severity: &str,
    message: &str,
    context: serde_json::Value,
) {
    if let Err(error) = sqlx::query(
        r#"
        INSERT INTO error_events (service, severity, message, context)
        VALUES ('chain-indexer', $1, $2, $3)
        "#,
    )
    .bind(severity)
    .bind(message)
    .bind(context)
    .execute(pool)
    .await
    {
        eprintln!("failed to persist error event: {error}");
    }
}

async fn index_chain_data(
    pool: &PgPool,
    chain_file: &PathBuf,
    chain_data: &str,
) -> Result<IndexResult> {
    let chain = parse_chain(chain_file, chain_data)?;
    validate_chain(&chain).context("chain validation failed; refusing to index invalid data")?;
    reindex_chain(pool, &chain).await
}

fn print_index_result(status: &str, result: IndexResult) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "status": status,
            "indexed_blocks": result.blocks,
            "indexed_transactions": result.transactions,
            "indexed_accounts": result.accounts,
            "indexed_rollup_batches": result.rollup_batches,
            "last_indexed_height": result.last_indexed_height,
        }))?
    );

    Ok(())
}

async fn reindex_chain(pool: &PgPool, chain: &Chain) -> Result<IndexResult> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to start index transaction")?;

    sqlx::query("DELETE FROM rollup_batch_transactions")
        .execute(&mut *tx)
        .await
        .context("failed to clear rollup batch transactions")?;
    sqlx::query("DELETE FROM rollup_batches")
        .execute(&mut *tx)
        .await
        .context("failed to clear rollup batches")?;
    sqlx::query("DELETE FROM transactions")
        .execute(&mut *tx)
        .await
        .context("failed to clear transactions")?;
    sqlx::query("DELETE FROM blocks")
        .execute(&mut *tx)
        .await
        .context("failed to clear blocks")?;
    sqlx::query("DELETE FROM accounts")
        .execute(&mut *tx)
        .await
        .context("failed to clear accounts")?;
    sqlx::query("DELETE FROM indexer_checkpoints WHERE id = 'chain'")
        .execute(&mut *tx)
        .await
        .context("failed to clear indexer checkpoint")?;

    let mut transaction_count = 0usize;
    for block in &chain.blocks {
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
        .bind(to_i32(block.transactions.len(), "transaction count")?)
        .bind(serde_json::to_value(block)?)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to insert block {}", block.height))?;

        for (transaction_index, transaction) in block.transactions.iter().enumerate() {
            insert_transaction(
                &mut tx,
                &transaction_hash(transaction),
                Some(block.height),
                transaction_index,
                transaction,
            )
            .await?;
            transaction_count += 1;
        }
    }

    for (transaction_index, transaction) in chain.pending_transactions.iter().enumerate() {
        insert_transaction(
            &mut tx,
            &transaction_hash(transaction),
            None,
            transaction_index,
            transaction,
        )
        .await?;
        transaction_count += 1;
    }

    for account in chain.accounts.values() {
        sqlx::query(
            r#"
            INSERT INTO accounts (address, balance, nonce)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(&account.address)
        .bind(to_i64(account.balance, "account balance")?)
        .bind(to_i64(account.nonce, "account nonce")?)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("failed to insert account {}", account.address))?;
    }

    let last_indexed_height = chain.blocks.last().map(|block| block.height);
    let rollup_batches = if let Some(end_height) = last_indexed_height {
        if end_height >= 1 {
            insert_rollup_batch(&mut tx, chain, 0, 1, end_height).await?;
            1
        } else {
            0
        }
    } else {
        0
    };

    sqlx::query(
        r#"
        INSERT INTO indexer_checkpoints (id, last_indexed_height)
        VALUES ('chain', $1)
        "#,
    )
    .bind(match last_indexed_height {
        Some(height) => Some(to_i64(height, "last indexed height")?),
        None => None,
    })
    .execute(&mut *tx)
    .await
    .context("failed to write indexer checkpoint")?;

    tx.commit()
        .await
        .context("failed to commit index transaction")?;

    Ok(IndexResult {
        blocks: chain.blocks.len(),
        transactions: transaction_count,
        accounts: chain.accounts.len(),
        rollup_batches,
        last_indexed_height,
    })
}

async fn insert_rollup_batch(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    chain: &Chain,
    batch_number: u64,
    start_height: u64,
    end_height: u64,
) -> Result<()> {
    let commitment = rollup_batch_commitment(chain, batch_number, start_height, end_height)
        .context("failed to build rollup batch commitment")?;

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
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'built', $10)
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
    .bind(to_i32(
        commitment.transaction_count,
        "rollup batch transaction count",
    )?)
    .bind(serde_json::to_value(&commitment)?)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("failed to insert rollup batch {batch_number}"))?;

    for block in chain
        .blocks
        .iter()
        .filter(|block| block.height >= start_height && block.height <= end_height)
    {
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
            .bind(to_i64(batch_number, "rollup batch number")?)
            .bind(&hash)
            .bind(to_i64(
                block.height,
                "rollup batch transaction block height",
            )?)
            .bind(to_i32(transaction_index, "rollup batch transaction index")?)
            .execute(&mut **tx)
            .await
            .with_context(|| {
                format!("failed to link transaction {hash} to rollup batch {batch_number}")
            })?;
        }
    }

    Ok(())
}

async fn insert_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    hash: &str,
    block_height: Option<u64>,
    transaction_index: usize,
    transaction: &chain_core::Transfer,
) -> Result<()> {
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
    .bind(match block_height {
        Some(height) => Some(to_i64(height, "transaction block height")?),
        None => None,
    })
    .bind(to_i32(transaction_index, "transaction index")?)
    .bind(&transaction.from)
    .bind(&transaction.to)
    .bind(to_i64(transaction.amount, "transaction amount")?)
    .bind(to_i64(transaction.nonce, "transaction nonce")?)
    .bind(&transaction.public_key_base64)
    .bind(&transaction.signature_base64)
    .bind(serde_json::to_value(transaction)?)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("failed to insert transaction {hash}"))?;

    Ok(())
}

fn to_i64(value: u64, label: &str) -> Result<i64> {
    i64::try_from(value).with_context(|| format!("{label} exceeds Postgres BIGINT range"))
}

fn to_i32(value: usize, label: &str) -> Result<i32> {
    i32::try_from(value).with_context(|| format!("{label} exceeds Postgres INTEGER range"))
}
