use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use chain_core::sign_transfer;
use clap::{Parser, Subcommand};
use wallet_core::{create_keypair, keypair_from_secret, validate_keypair, DevKeypair};

#[derive(Debug, Parser)]
#[command(name = "wallet", about = "Development Ethereum wallet for the rollup")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Create {
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Load {
        #[arg(long)]
        key_file: PathBuf,
    },
    Import {
        #[arg(long)]
        secret_key: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Address {
        #[arg(long)]
        key_file: PathBuf,
    },
    SignTransfer {
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long)]
        to: String,
        #[arg(long)]
        amount: u64,
        #[arg(long)]
        nonce: u64,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Create { output } => write_or_print(&create_keypair(), output)?,
        Command::Load { key_file } => print_keypair(&read_keypair(&key_file)?)?,
        Command::Import { secret_key, output } => {
            write_or_print(&keypair_from_secret(&secret_key)?, output)?
        }
        Command::Address { key_file } => println!("{}", read_keypair(&key_file)?.address),
        Command::SignTransfer {
            key_file,
            to,
            amount,
            nonce,
        } => {
            let wallet = read_keypair(&key_file)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&sign_transfer(
                    &wallet.secret_key_hex,
                    to,
                    amount,
                    nonce
                )?)?
            );
        }
    }
    Ok(())
}

fn write_or_print(wallet: &DevKeypair, output: Option<PathBuf>) -> Result<()> {
    let json = serde_json::to_string_pretty(wallet)?;
    if let Some(path) = output {
        fs::write(&path, format!("{json}\n"))
            .with_context(|| format!("failed to write key file {}", path.display()))?;
        println!("{}", path.display());
    } else {
        println!("{json}");
    }
    Ok(())
}

fn print_keypair(wallet: &DevKeypair) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(wallet)?);
    Ok(())
}

fn read_keypair(path: &PathBuf) -> Result<DevKeypair> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read key file {}", path.display()))?;
    let wallet: DevKeypair = serde_json::from_str(&data)
        .with_context(|| format!("failed to parse key file {}", path.display()))?;
    validate_keypair(&wallet).with_context(|| format!("invalid key file {}", path.display()))?;
    Ok(wallet)
}
