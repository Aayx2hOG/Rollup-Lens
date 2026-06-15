use std::{fs, path::PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use merkle_core::{create_proof, root_hex, verify_proof, MerkleProof};
use serde_json::Value;

#[derive(Debug, Parser)]
#[command(name = "merkle")]
#[command(about = "Merkle tree commitments and inclusion proofs for JSON leaves")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Root {
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },

    Prove {
        #[arg(value_name = "FILE")]
        file: PathBuf,
        #[arg(value_name = "INDEX")]
        index: usize,
    },

    Verify {
        #[arg(long, value_name = "JSON")]
        leaf: String,
        #[arg(long, value_name = "FILE")]
        proof: PathBuf,
        #[arg(long, value_name = "HEX")]
        root: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Root { file } => {
            let leaves = read_leaf_file(&file)?;
            println!("{}", root_hex(&leaves));
        }
        Command::Prove { file, index } => {
            let leaves = read_leaf_file(&file)?;
            let proof = create_proof(&leaves, index)?;
            println!("{}", serde_json::to_string_pretty(&proof)?);
        }
        Command::Verify { leaf, proof, root } => {
            let leaf = parse_leaf(&leaf)?;
            let proof = read_proof_file(&proof)?;
            println!("{}", verify_proof(&leaf, &proof, &root)?);
        }
    }

    Ok(())
}

fn read_leaf_file(path: &PathBuf) -> Result<Vec<Value>> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read leaf file {}", path.display()))?;
    let value: Value = serde_json::from_str(&data)
        .with_context(|| format!("failed to parse JSON leaf file {}", path.display()))?;
    match value {
        Value::Array(values) => Ok(values),
        _ => bail!("leaf file must contain a JSON array"),
    }
}

fn parse_leaf(input: &str) -> Result<Value> {
    serde_json::from_str(input).context("failed to parse --leaf JSON")
}

fn read_proof_file(path: &PathBuf) -> Result<MerkleProof> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read proof file {}", path.display()))?;
    serde_json::from_str(&data)
        .with_context(|| format!("failed to parse proof file {}", path.display()))
}
