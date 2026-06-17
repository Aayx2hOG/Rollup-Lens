use std::{env, fs, path::PathBuf};

use anyhow::{Context, Result};
use chain_core::{transaction_hash, validate_chain, Chain};
use clap::Parser;
use sqlx::{postgres::PgPoolOptions, Executor, PgPool};

const INIT_SQL: &str = include_str!("../../../prisma/migrations/0001_init/migration.sql");
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
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let database_url = database_url(cli.database_url)?;
    let chain = read_chain(&cli.chain_file)?;
    validate_chain(&chain).context("chain validation failed; refusing to index invalid data")?;

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .context("failed to connect to Postgres")?;

    if cli.init_schema {
        init_schema(&pool).await?;
    }

    let result = reindex_chain(&pool, &chain).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "indexed_blocks": result.blocks,
            "indexed_transactions": result.transactions,
            "indexed_accounts": result.accounts,
            "last_indexed_height": result.last_indexed_height,
        }))?
    );

    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct IndexResult {
    blocks: usize,
    transactions: usize,
    accounts: usize,
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

fn read_chain(path: &PathBuf) -> Result<Chain> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read chain file {}", path.display()))?;
    serde_json::from_str(&data)
        .with_context(|| format!("failed to parse chain file {}", path.display()))
}

async fn init_schema(pool: &PgPool) -> Result<()> {
    for statement in INIT_SQL
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
    {
        pool.execute(statement)
            .await
            .with_context(|| format!("failed to execute schema statement: {statement}"))?;
    }

    Ok(())
}

async fn reindex_chain(pool: &PgPool, chain: &Chain) -> Result<IndexResult> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to start index transaction")?;

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
        last_indexed_height,
    })
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
