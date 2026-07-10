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

Print the verification-key commitment used when deploying the settlement
contract:

```bash
cargo run --release -p validity-prover -- vkey
```

Generate and locally verify a core proof:

```bash
cargo run --release -p validity-prover -- \
  prove witness.json --proof-out batch-proof.bin
```

Generate a Groth16 proof in the encoding accepted by the official SP1 Solidity
verifier:

```bash
cargo run --release -p validity-prover -- \
  groth16 witness.json --evm-artifact-out batch-proof.json
```

For a batch produced by the explorer sequencer, export its persisted witness:

```bash
curl http://127.0.0.1:3000/rollup/batches/0/validity-witness \
  > witness.json
```

The JSON artifact contains:

- `program_vkey`: the verification-key commitment that must equal the
  `programVKey` configured in `ValidityRollupSettlement`
- `public_values`: 448 bytes containing the ABI encoding of batch number, five
  roots/commitments, deposit root, processed deposit range/count, transaction
  count, withdrawal root/count, and the L1 deposit inbox
- `proof`: the selector-prefixed Groth16 proof bytes expected by SP1's
  version-matched Solidity verifier
- `decoded`: a human-readable copy for inspection

The contract binds the first six public values to the batch arguments and checks
the encoded L1 deposit inbox before calling the SP1 verifier. Consequently, a
proof generated for one batch or deposit inbox cannot be submitted for another.

Groth16 wrapping is substantially slower than `execute` or a core proof. Use
the Succinct prover network or a machine configured for SP1 proving for
production-sized batches.
