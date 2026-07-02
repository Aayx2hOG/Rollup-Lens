use std::{fs, path::PathBuf};

use anyhow::{bail, Context, Result};
use chain_core::sign_transfer;
use clap::{Parser, Subcommand};
use wallet_core::{
    address_from_public_key, create_keypair, keypair_from_secret, public_key_from_secret,
    sign_message, validate_keypair, verify_message, DevKeypair,
};

#[derive(Debug, Parser)]
#[command(name = "wallet")]
#[command(about = "Development wallet primitives for the zk-rollup portfolio")]
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
        #[arg(
            long,
            conflicts_with = "key_file",
            required_unless_present = "key_file"
        )]
        public_key: Option<String>,
        #[arg(
            long,
            conflicts_with = "public_key",
            required_unless_present = "public_key"
        )]
        key_file: Option<PathBuf>,
    },

    PublicKey {
        #[arg(
            long,
            conflicts_with = "key_file",
            required_unless_present = "key_file"
        )]
        secret_key: Option<String>,
        #[arg(
            long,
            conflicts_with = "secret_key",
            required_unless_present = "secret_key"
        )]
        key_file: Option<PathBuf>,
    },

    Sign {
        #[arg(
            long,
            conflicts_with = "key_file",
            required_unless_present = "key_file"
        )]
        secret_key: Option<String>,
        #[arg(
            long,
            conflicts_with = "secret_key",
            required_unless_present = "secret_key"
        )]
        key_file: Option<PathBuf>,
        #[arg(long)]
        message: String,
    },

    Verify {
        #[arg(long)]
        public_key: String,
        #[arg(long)]
        message: String,
        #[arg(long)]
        signature: String,
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
    let cli = Cli::parse();

    match cli.command {
        Command::Create { output } => {
            let keypair = create_keypair();
            write_or_print_keypair(&keypair, output)?;
        }
        Command::Load { key_file } => {
            let keypair = read_keypair(&key_file)?;
            println!("{}", serde_json::to_string_pretty(&keypair)?);
        }
        Command::Import { secret_key, output } => {
            let keypair = keypair_from_secret(&secret_key)?;
            write_or_print_keypair(&keypair, output)?;
        }
        Command::Address {
            public_key,
            key_file,
        } => {
            let public_key = public_key_from_inputs(public_key, key_file)?;
            println!("{}", address_from_public_key(&public_key)?);
        }
        Command::PublicKey {
            secret_key,
            key_file,
        } => {
            let secret_key = secret_key_from_inputs(secret_key, key_file)?;
            println!("{}", public_key_from_secret(&secret_key)?);
        }
        Command::Sign {
            secret_key,
            key_file,
            message,
        } => {
            let secret_key = secret_key_from_inputs(secret_key, key_file)?;
            let signed = sign_message(&secret_key, &message)?;
            println!("{}", serde_json::to_string_pretty(&signed)?);
        }
        Command::Verify {
            public_key,
            message,
            signature,
        } => {
            println!("{}", verify_message(&public_key, &message, &signature)?);
        }
        Command::SignTransfer {
            key_file,
            to,
            amount,
            nonce,
        } => {
            let keypair = read_keypair(&key_file)?;
            let transfer = sign_transfer(&keypair.secret_key_base64, to, amount, nonce)?;
            println!("{}", serde_json::to_string_pretty(&transfer)?);
        }
    }

    Ok(())
}

fn write_or_print_keypair(keypair: &DevKeypair, output: Option<PathBuf>) -> Result<()> {
    let json = serde_json::to_string_pretty(keypair)?;
    if let Some(path) = output {
        fs::write(&path, format!("{json}\n"))
            .with_context(|| format!("failed to write key file {}", path.display()))?;
        println!("{}", path.display());
    } else {
        println!("{json}");
    }

    Ok(())
}

fn read_keypair(path: &PathBuf) -> Result<DevKeypair> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read key file {}", path.display()))?;
    let keypair: DevKeypair = serde_json::from_str(&data)
        .with_context(|| format!("failed to parse key file {}", path.display()))?;
    validate_keypair(&keypair).with_context(|| format!("invalid key file {}", path.display()))?;

    Ok(keypair)
}

fn secret_key_from_inputs(secret_key: Option<String>, key_file: Option<PathBuf>) -> Result<String> {
    match (secret_key, key_file) {
        (Some(secret_key), None) => Ok(secret_key),
        (None, Some(path)) => Ok(read_keypair(&path)?.secret_key_base64),
        _ => bail!("provide exactly one of --secret-key or --key-file"),
    }
}

fn public_key_from_inputs(public_key: Option<String>, key_file: Option<PathBuf>) -> Result<String> {
    match (public_key, key_file) {
        (Some(public_key), None) => Ok(public_key),
        (None, Some(path)) => Ok(read_keypair(&path)?.public_key_base64),
        _ => bail!("provide exactly one of --public-key or --key-file"),
    }
}
