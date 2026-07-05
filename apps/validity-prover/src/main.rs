use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use serde::Serialize;
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
    witness: Option<PathBuf>,
    #[arg(long, value_name = "PROOF_FILE")]
    proof_out: Option<PathBuf>,
    #[arg(long, value_name = "ARTIFACT_JSON")]
    evm_artifact_out: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Mode {
    Execute,
    Prove,
    Groth16,
    Vkey,
}

#[derive(Serialize)]
struct EvmProofArtifact {
    program_vkey: String,
    public_values: String,
    proof: String,
    decoded: BatchValidityPublicValues,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let client = ProverClient::from_env();
    if matches!(args.mode, Mode::Vkey) {
        let proving_key = client
            .setup(VALIDITY_GUEST_ELF)
            .context("failed to set up validity guest")?;
        println!("{}", proving_key.verifying_key().bytes32());
        return Ok(());
    }
    let witness = args
        .witness
        .as_ref()
        .context("WITNESS_JSON is required unless vkey mode is used")?;
    let witness: BatchValidityWitness = serde_json::from_slice(
        &fs::read(witness).with_context(|| format!("failed to read {}", witness.display()))?,
    )
    .context("invalid batch witness JSON")?;

    let mut stdin = SP1Stdin::new();
    stdin.write(&witness);

    match args.mode {
        Mode::Execute => {
            let (output, report) = client
                .execute(VALIDITY_GUEST_ELF, stdin)
                .run()
                .context("zkVM execution failed")?;
            let values = validity_core::verify_batch_witness(&witness)
                .context("invalid validity witness")?;
            let expected = values
                .encode_evm()
                .context("failed to encode public values")?;
            anyhow::ensure!(
                output.as_slice() == expected,
                "guest public values differ from host"
            );
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
        Mode::Groth16 => {
            let artifact_out = args
                .evm_artifact_out
                .as_ref()
                .context("--evm-artifact-out is required in groth16 mode")?;
            let proving_key = client
                .setup(VALIDITY_GUEST_ELF)
                .context("failed to set up validity guest")?;
            let proof = client
                .prove(&proving_key, stdin)
                .groth16()
                .run()
                .context("failed to generate Groth16 proof")?;
            client
                .verify(&proof, proving_key.verifying_key(), None)
                .context("generated Groth16 proof did not verify")?;
            let decoded = validity_core::verify_batch_witness(&witness)
                .context("invalid validity witness")?;
            let expected = decoded
                .encode_evm()
                .context("failed to encode public values")?;
            anyhow::ensure!(
                proof.public_values.as_slice() == expected,
                "proof public values differ from the expected Solidity ABI encoding"
            );
            let artifact = EvmProofArtifact {
                program_vkey: proving_key.verifying_key().bytes32(),
                public_values: encode_hex(proof.public_values.as_slice()),
                proof: encode_hex(&proof.bytes()),
                decoded,
            };
            fs::write(artifact_out, serde_json::to_vec_pretty(&artifact)?)
                .with_context(|| format!("failed to write {}", artifact_out.display()))?;
            println!("wrote EVM proof artifact to {}", artifact_out.display());
        }
        Mode::Vkey => unreachable!("vkey mode exits before reading a witness"),
    }

    Ok(())
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(2 + bytes.len() * 2);
    output.push_str("0x");
    for byte in bytes {
        use std::fmt::Write;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
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

        let (output, _) = client.execute(VALIDITY_GUEST_ELF, stdin).run().unwrap();
        let values = validity_core::verify_batch_witness(&witness).unwrap();
        assert_eq!(values.new_state_root, expected_new_root);
        assert_eq!(output.as_slice(), values.encode_evm().unwrap());
    }
}
