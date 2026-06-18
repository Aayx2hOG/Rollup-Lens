use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use chain_core::{
    add_genesis_account, add_pending_transaction, finalize_genesis, new_chain, next_nonce,
    produce_block, rollup_batch_commitment, sign_transfer, state_root, validate_chain, Chain,
};
use clap::{Parser, Subcommand};
use wallet_core::{address_from_public_key, create_keypair, validate_keypair, DevKeypair};

#[derive(Debug, Parser)]
#[command(name = "chain")]
#[command(about = "Minimal blockchain state machine for the zk-rollup portfolio")]
struct Cli {
    #[arg(long, default_value = "chain.json", global = true)]
    chain_file: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init,

    Account {
        #[command(subcommand)]
        command: AccountCommand,
    },

    Genesis {
        #[command(subcommand)]
        command: GenesisCommand,
    },

    Tx {
        #[command(subcommand)]
        command: TxCommand,
    },

    Block {
        #[command(subcommand)]
        command: BlockCommand,
    },

    Rollup {
        #[command(subcommand)]
        command: RollupCommand,
    },

    State,

    Validate,
}

#[derive(Debug, Subcommand)]
enum AccountCommand {
    Create {
        #[arg(long)]
        balance: u64,
        #[arg(long)]
        key_output: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum GenesisCommand {
    Finalize,
}

#[derive(Debug, Subcommand)]
enum TxCommand {
    Send {
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long)]
        to: String,
        #[arg(long)]
        amount: u64,
    },
}

#[derive(Debug, Subcommand)]
enum BlockCommand {
    Produce,
}

#[derive(Debug, Subcommand)]
enum RollupCommand {
    Batch {
        #[arg(long)]
        batch_number: u64,
        #[arg(long)]
        start_height: u64,
        #[arg(long)]
        end_height: u64,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Init => {
            ensure_chain_does_not_exist(&cli.chain_file)?;
            let chain = new_chain(Vec::new())?;
            write_chain(&cli.chain_file, &chain)?;
            println!("{}", cli.chain_file.display());
        }
        Command::Account {
            command:
                AccountCommand::Create {
                    balance,
                    key_output,
                },
        } => {
            let mut chain = read_chain(&cli.chain_file)?;
            let keypair = create_keypair();
            let address = address_from_public_key(&keypair.public_key_base64)?;
            add_genesis_account(&mut chain, address.clone(), balance)?;
            write_chain(&cli.chain_file, &chain)?;
            write_keypair_if_requested(&keypair, key_output)?;

            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "address": address,
                    "balance": balance,
                    "public_key_base64": keypair.public_key_base64,
                }))?
            );
        }
        Command::Genesis {
            command: GenesisCommand::Finalize,
        } => {
            let mut chain = read_chain(&cli.chain_file)?;
            let block = finalize_genesis(&mut chain)?;
            write_chain(&cli.chain_file, &chain)?;
            println!("{}", serde_json::to_string_pretty(&block)?);
        }
        Command::Tx {
            command:
                TxCommand::Send {
                    key_file,
                    to,
                    amount,
                },
        } => {
            let mut chain = read_chain(&cli.chain_file)?;
            let keypair = read_keypair(&key_file)?;
            let from = address_from_public_key(&keypair.public_key_base64)?;
            let nonce = next_nonce(&chain, &from)?;
            let transaction = sign_transfer(&keypair.secret_key_base64, to, amount, nonce)?;
            add_pending_transaction(&mut chain, transaction.clone())?;
            write_chain(&cli.chain_file, &chain)?;
            println!("{}", serde_json::to_string_pretty(&transaction)?);
        }
        Command::Block {
            command: BlockCommand::Produce,
        } => {
            let mut chain = read_chain(&cli.chain_file)?;
            let block = produce_block(&mut chain)?;
            write_chain(&cli.chain_file, &chain)?;
            println!("{}", serde_json::to_string_pretty(&block)?);
        }
        Command::Rollup {
            command:
                RollupCommand::Batch {
                    batch_number,
                    start_height,
                    end_height,
                },
        } => {
            let chain = read_chain(&cli.chain_file)?;
            let batch = rollup_batch_commitment(&chain, batch_number, start_height, end_height)?;
            println!("{}", serde_json::to_string_pretty(&batch)?);
        }
        Command::State => {
            let chain = read_chain(&cli.chain_file)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "state_root": state_root(&chain.accounts),
                    "genesis_finalized": !chain.blocks.is_empty(),
                    "accounts": chain.accounts,
                    "pending_transactions": chain.pending_transactions,
                    "block_count": chain.blocks.len(),
                }))?
            );
        }
        Command::Validate => {
            let chain = read_chain(&cli.chain_file)?;
            validate_chain(&chain)?;
            println!("true");
        }
    }

    Ok(())
}

fn ensure_chain_does_not_exist(path: &Path) -> Result<()> {
    if path.exists() {
        bail!("chain file {} already exists", path.display());
    }
    Ok(())
}

fn read_chain(path: &Path) -> Result<Chain> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read chain file {}", path.display()))?;
    serde_json::from_str(&data)
        .with_context(|| format!("failed to parse chain file {}", path.display()))
}

fn write_chain(path: &Path, chain: &Chain) -> Result<()> {
    let json = serde_json::to_string_pretty(chain)?;
    fs::write(path, format!("{json}\n"))
        .with_context(|| format!("failed to write chain file {}", path.display()))
}

fn read_keypair(path: &Path) -> Result<DevKeypair> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read key file {}", path.display()))?;
    let keypair: DevKeypair = serde_json::from_str(&data)
        .with_context(|| format!("failed to parse key file {}", path.display()))?;
    validate_keypair(&keypair).with_context(|| format!("invalid key file {}", path.display()))?;
    Ok(keypair)
}

fn write_keypair_if_requested(keypair: &DevKeypair, output: Option<PathBuf>) -> Result<()> {
    if let Some(path) = output {
        let json = serde_json::to_string_pretty(keypair)?;
        fs::write(&path, format!("{json}\n"))
            .with_context(|| format!("failed to write key file {}", path.display()))?;
    }
    Ok(())
}
