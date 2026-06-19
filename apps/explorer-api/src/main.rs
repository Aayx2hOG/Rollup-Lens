use std::{env, fs, net::SocketAddr, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use chain_core::{
    add_pending_transaction, finalize_genesis, new_chain, next_nonce, produce_block, sign_transfer,
    Account, Chain,
};
use clap::Parser;
use explorer_api::{app, app_with_pool};
use sqlx::postgres::PgPoolOptions;
use wallet_core::{address_from_public_key, create_keypair, DevKeypair};

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

    #[arg(long)]
    auto_demo: bool,

    #[arg(long, default_value = "/tmp/rollup-lens-live-chain.json")]
    live_chain_file: PathBuf,

    #[arg(long, default_value_t = 10)]
    auto_demo_interval_seconds: u64,

    #[arg(long, default_value = "0.0.0.0:3000")]
    bind: SocketAddr,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let listener = tokio::net::TcpListener::bind(cli.bind)
        .await
        .with_context(|| format!("failed to bind explorer API to {}", cli.bind))?;

    let router = if cli.auto_demo {
        if cli.auto_demo_interval_seconds == 0 {
            anyhow::bail!("--auto-demo-interval-seconds must be greater than zero");
        }

        start_auto_demo_chain(
            cli.live_chain_file.clone(),
            Duration::from_secs(cli.auto_demo_interval_seconds),
        )?;
        println!(
            "serving auto-demo live chain file {}",
            cli.live_chain_file.display()
        );
        app(cli.live_chain_file)
    } else if cli.json_mode {
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

fn start_auto_demo_chain(chain_file: PathBuf, interval: Duration) -> Result<()> {
    let mut demo = LiveDemoChain::new()?;
    write_chain_atomic(&chain_file, &demo.chain)?;

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            if let Err(error) = demo.advance_and_write(&chain_file) {
                eprintln!("auto-demo chain update failed: {error:#}");
            }
        }
    });

    Ok(())
}

struct LiveDemoChain {
    chain: Chain,
    alice: DevKeypair,
    bob: DevKeypair,
    alice_address: String,
    bob_address: String,
    sequence: u64,
}

impl LiveDemoChain {
    fn new() -> Result<Self> {
        let alice = create_keypair();
        let bob = create_keypair();
        let alice_address = address_from_public_key(&alice.public_key_base64)?;
        let bob_address = address_from_public_key(&bob.public_key_base64)?;
        let mut chain = new_chain(vec![
            Account {
                address: alice_address.clone(),
                balance: 1_000,
                nonce: 0,
            },
            Account {
                address: bob_address.clone(),
                balance: 1_000,
                nonce: 0,
            },
        ])?;
        finalize_genesis(&mut chain)?;

        Ok(Self {
            chain,
            alice,
            bob,
            alice_address,
            bob_address,
            sequence: 0,
        })
    }

    fn advance_and_write(&mut self, chain_file: &PathBuf) -> Result<()> {
        self.advance()?;
        write_chain_atomic(chain_file, &self.chain)
    }

    fn advance(&mut self) -> Result<()> {
        let alice_sends = self.sequence % 2 == 0;
        let amount = 1 + (self.sequence % 5);
        let (secret_key, from, to) = if alice_sends {
            (
                &self.alice.secret_key_base64,
                &self.alice_address,
                self.bob_address.clone(),
            )
        } else {
            (
                &self.bob.secret_key_base64,
                &self.bob_address,
                self.alice_address.clone(),
            )
        };
        let nonce = next_nonce(&self.chain, from)?;
        let transfer = sign_transfer(secret_key, to, amount, nonce)?;
        add_pending_transaction(&mut self.chain, transfer)?;
        produce_block(&mut self.chain)?;
        self.sequence += 1;
        Ok(())
    }
}

fn write_chain_atomic(path: &PathBuf, chain: &Chain) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create chain directory {}", parent.display()))?;
    }

    let tmp_path = path.with_extension("json.tmp");
    let data = serde_json::to_string_pretty(chain).context("failed to serialize live chain")?;
    fs::write(&tmp_path, data).with_context(|| {
        format!(
            "failed to write temporary chain file {}",
            tmp_path.display()
        )
    })?;
    fs::rename(&tmp_path, path)
        .with_context(|| format!("failed to replace live chain file {}", path.display()))?;
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
