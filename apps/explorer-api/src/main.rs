use std::{env, net::SocketAddr, time::Duration};

use anyhow::{Context, Result};
use clap::Parser;
use explorer_api::{app_with_pool, sequence_pending_transactions};
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
    auto_demo: bool,

    #[arg(long, default_value_t = 10)]
    auto_demo_interval_seconds: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let listener = tokio::net::TcpListener::bind(cli.bind)
        .await
        .with_context(|| format!("failed to bind explorer API to {}", cli.bind))?;

    let database_url = database_url(cli.database_url)?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .context("failed to connect to Postgres")?;
    if cli.auto_demo {
        spawn_auto_demo(pool.clone(), cli.auto_demo_interval_seconds);
    }

    let router = app_with_pool(pool);

    println!("serving Postgres-backed rollup API");
    println!("listening on http://{}", cli.bind);
    axum::serve(listener, router).await?;
    Ok(())
}

fn spawn_auto_demo(pool: sqlx::PgPool, interval_seconds: u64) {
    let interval = Duration::from_secs(interval_seconds.max(1));
    tokio::spawn(async move {
        println!(
            "auto demo sequencer enabled; checking pending transactions every {} seconds",
            interval.as_secs()
        );
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            match sequence_pending_transactions(pool.clone(), 100).await {
                Ok(true) => println!("auto demo sequenced pending transactions"),
                Ok(false) => {}
                Err(error) => eprintln!("auto demo sequencing failed: {error}"),
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
