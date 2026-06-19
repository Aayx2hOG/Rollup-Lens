use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use explorer_api::{app, app_with_pool};
use sqlx::postgres::PgPoolOptions;

const PRISMA_ENV_PATH: &str = "prisma/.env";

#[derive(Debug, Parser)]
#[command(name = "explorer-api")]
#[command(
    about = "Read-only blockchain explorer API for local chain JSON or indexed Postgres data"
)]
struct Cli {
    #[arg(long, default_value = "chain.json")]
    chain_file: PathBuf,

    #[arg(long)]
    database_url: Option<String>,

    #[arg(long)]
    json_mode: bool,

    #[arg(long, default_value = "0.0.0.0:3000")]
    bind: SocketAddr,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let listener = tokio::net::TcpListener::bind(cli.bind)
        .await
        .with_context(|| format!("failed to bind explorer API to {}", cli.bind))?;

    let router = if cli.json_mode {
        println!("serving chain JSON file {}", cli.chain_file.display());
        app(cli.chain_file)
    } else if let Some(database_url) = database_url(cli.database_url) {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .context("failed to connect to Postgres")?;
        println!("serving indexed Postgres data");
        app_with_pool(pool)
    } else {
        println!("serving chain JSON file {}", cli.chain_file.display());
        app(cli.chain_file)
    };

    println!("listening on http://{}", cli.bind);
    axum::serve(listener, router).await?;
    Ok(())
}

fn database_url(cli_value: Option<String>) -> Option<String> {
    cli_value
        .or_else(|| env::var("DATABASE_URL").ok())
        .or_else(database_url_from_prisma_env)
        .filter(|value| !value.trim().is_empty())
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
