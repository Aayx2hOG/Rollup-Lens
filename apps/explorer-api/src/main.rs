use std::{env, fs, net::SocketAddr, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use clap::Parser;
use explorer_api::{
    app_with_pool, initialize_genesis, sequence_pending_transactions, GenesisConfig,
};
use sqlx::postgres::PgPoolOptions;

const PRISMA_ENV_PATH: &str = "prisma/.env";

#[derive(Debug, Parser)]
#[command(name = "explorer-api")]
#[command(about = "Postgres-backed rollup explorer and sequencer API")]
struct Cli {
    #[arg(long)]
    database_url: Option<String>,

    #[arg(long, default_value = "0.0.0.0:3000")]
    bind: SocketAddr,

    #[arg(long)]
    auto_sequence: bool,

    #[arg(long, default_value_t = 10)]
    auto_sequence_interval_seconds: u64,

    #[arg(
        long,
        value_name = "GENESIS_JSON",
        conflicts_with = "auto_sequence",
        help = "Initialize genesis atomically from an account allocation file, then exit"
    )]
    initialize_genesis: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let database_url = database_url(cli.database_url)?;
    validate_database_url(&database_url)?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_lazy(&database_url)
        .context("DATABASE_URL is not a valid Postgres connection string")?;

    if let Some(path) = &cli.initialize_genesis {
        let config: GenesisConfig = serde_json::from_slice(
            &fs::read(path)
                .with_context(|| format!("failed to read genesis file {}", path.display()))?,
        )
        .with_context(|| format!("failed to parse genesis file {}", path.display()))?;
        let initialized = initialize_genesis(&pool, config)
            .await
            .context("genesis initialization failed")?;
        println!("{}", serde_json::to_string_pretty(&initialized)?);
        return Ok(());
    }

    let listener = tokio::net::TcpListener::bind(cli.bind)
        .await
        .with_context(|| format!("failed to bind explorer API to {}", cli.bind))?;
    if cli.auto_sequence {
        spawn_auto_sequencer(pool.clone(), cli.auto_sequence_interval_seconds);
    }

    let router = app_with_pool(pool);

    println!("serving Postgres-backed rollup API");
    println!("listening on http://{}", cli.bind);
    println!("readiness probe available at /ready");
    axum::serve(listener, router).await?;
    Ok(())
}

fn spawn_auto_sequencer(pool: sqlx::PgPool, interval_seconds: u64) {
    let interval = Duration::from_secs(interval_seconds.max(1));
    tokio::spawn(async move {
        println!(
            "automatic sequencer enabled; checking pending transactions every {} seconds",
            interval.as_secs()
        );
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            match sequence_pending_transactions(pool.clone(), 100).await {
                Ok(true) => println!("automatic sequencer processed pending transactions"),
                Ok(false) => {}
                Err(error) => eprintln!("automatic sequencing failed: {error}"),
            }
        }
    });
}

fn database_url(cli_value: Option<String>) -> Result<String> {
    cli_value
        .or_else(|| env::var("DATABASE_URL").ok())
        .or_else(database_url_from_prisma_env)
        .filter(|value| !value.trim().is_empty())
        .context("provide --database-url, set DATABASE_URL, or add DATABASE_URL to prisma/.env")
}

fn validate_database_url(database_url: &str) -> Result<()> {
    if !database_url.starts_with("postgres://") && !database_url.starts_with("postgresql://") {
        anyhow::bail!("DATABASE_URL must use the postgres:// or postgresql:// scheme");
    }
    if !database_url.contains('@') {
        anyhow::bail!("DATABASE_URL must include database credentials and a host");
    }
    Ok(())
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
