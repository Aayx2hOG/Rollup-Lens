# SP1 Batch Validity Prover

This application proves that a rollup batch was produced by the same
deterministic state transition implemented by `chain-core`.

## Proof statement

The private witness contains the complete pre-batch account state, the previous
block, and the ordered batch blocks. The guest rejects it unless:

- the pre-state hashes to the previous block's state root
- block heights, parent hashes, and block hashes form a valid chain
- every transaction has a valid signature, nonce, amount, and balance
- executing each block produces its claimed state root
- transaction, block, and batch-data commitments are deterministic

The proof exposes only the batch number, previous and new state roots,
transaction root, block-hash root, data hash, and transaction count.

## Usage

Install the SP1 toolchain:

```bash
curl -L https://sp1up.succinct.xyz | bash
sp1up
```

Execute a witness without proving:

```bash
cargo run --release -p validity-prover -- execute witness.json
```

Generate and locally verify a core proof:

```bash
cargo run --release -p validity-prover -- \
  prove witness.json --proof-out batch-proof.bin
```

Core proofs are not directly accepted by `RollupSettlement.sol`. Production L1
settlement additionally requires an EVM-compatible compressed proof, the
official SP1 verifier contract, and binding the public values to `submitBatch`.
