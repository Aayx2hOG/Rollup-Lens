use std::{net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use explorer_api::app;

#[derive(Debug, Parser)]
#[command(name = "explorer-api")]
#[command(about = "Read-only blockchain explorer API for local chain JSON files")]
struct Cli {
    #[arg(long, default_value = "chain.json")]
    chain_file: PathBuf,

    #[arg(long, default_value = "0.0.0.0:3000")]
    bind: SocketAddr,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let listener = tokio::net::TcpListener::bind(cli.bind)
        .await
        .with_context(|| format!("failed to bind explorer API to {}", cli.bind))?;

    println!("listening on http://{}", cli.bind);
    axum::serve(listener, app(cli.chain_file)).await?;
    Ok(())
}
