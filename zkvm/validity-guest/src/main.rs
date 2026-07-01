#![no_main]

sp1_zkvm::entrypoint!(main);

use validity_core::{verify_batch_witness, BatchValidityWitness};

pub fn main() {
    let witness = sp1_zkvm::io::read::<BatchValidityWitness>();
    let public_values =
        verify_batch_witness(&witness).expect("invalid rollup batch validity witness");
    let encoded = public_values
        .encode_evm()
        .expect("valid batch commitments must be EVM encodable");
    sp1_zkvm::io::commit_slice(&encoded);
}
