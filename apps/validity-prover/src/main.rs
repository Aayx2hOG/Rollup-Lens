use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use sp1_sdk::{
    blocking::{ProveRequest, Prover, ProverClient},
    include_elf, Elf, HashableKey, ProvingKey, SP1Stdin,
};
use validity_core::{BatchValidityPublicValues, BatchValidityWitness};

const VALIDITY_GUEST_ELF: Elf = include_elf!("validity-guest");

#[derive(Debug, Parser)]
#[command(about = "Execute or prove a zk-rollup batch state transition with SP1")]
struct Args {
    #[arg(value_enum)]
    mode: Mode,
    #[arg(value_name = "WITNESS_JSON")]
    witness: PathBuf,
    #[arg(long, value_name = "PROOF_FILE")]
    proof_out: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Mode {
    Execute,
    Prove,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let witness: BatchValidityWitness = serde_json::from_slice(
        &fs::read(&args.witness)
            .with_context(|| format!("failed to read {}", args.witness.display()))?,
    )
    .context("invalid batch witness JSON")?;

    let client = ProverClient::from_env();
    let mut stdin = SP1Stdin::new();
    stdin.write(&witness);

    match args.mode {
        Mode::Execute => {
            let (mut output, report) = client
                .execute(VALIDITY_GUEST_ELF, stdin)
                .run()
                .context("zkVM execution failed")?;
            let values: BatchValidityPublicValues = output.read();
            println!("{}", serde_json::to_string_pretty(&values)?);
            eprintln!("instructions: {}", report.total_instruction_count());
        }
        Mode::Prove => {
            let proof_out = args
                .proof_out
                .as_ref()
                .context("--proof-out is required in prove mode")?;
            let proving_key = client
                .setup(VALIDITY_GUEST_ELF)
                .context("failed to set up validity guest")?;
            let proof = client
                .prove(&proving_key, stdin)
                .run()
                .context("failed to generate validity proof")?;
            client
                .verify(&proof, proving_key.verifying_key(), None)
                .context("generated validity proof did not verify")?;
            proof
                .save(proof_out)
                .with_context(|| format!("failed to save {}", proof_out.display()))?;
            println!(
                "verified proof for program vkey {}",
                proving_key.verifying_key().bytes32()
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use chain_core::{finalize_genesis, new_chain, produce_block};

    use super::*;

    #[test]
    #[ignore = "full zkVM execution is intentionally opt-in because SHA-256 state-tree execution is expensive"]
    fn zkvm_executes_a_valid_batch_and_commits_its_roots() {
        let mut chain = new_chain(vec![]).unwrap();
        let previous_block = finalize_genesis(&mut chain).unwrap();
        let block = produce_block(&mut chain).unwrap();
        let expected_new_root = block.state_root.clone();

        let witness = BatchValidityWitness {
            batch_number: 0,
            accounts_before: Default::default(),
            previous_block,
            blocks: vec![block],
        };
        let client = ProverClient::from_env();
        let mut stdin = SP1Stdin::new();
        stdin.write(&witness);

        let (mut output, _) = client.execute(VALIDITY_GUEST_ELF, stdin).run().unwrap();
        let values: BatchValidityPublicValues = output.read();

        assert_eq!(values.new_state_root, expected_new_root);
        assert_eq!(values.transaction_count, 0);
    }
}
