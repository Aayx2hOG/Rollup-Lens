# EVM Settlement Client

`settlement-client` bridges the Rust rollup API to the Solidity
`RollupSettlement` contract. It fetches batch calldata from the explorer API and
broadcasts contract transactions through Foundry's `cast`.

## Prerequisites

- Explorer API running on `http://127.0.0.1:3000`
- An EVM RPC, for example Anvil on `http://127.0.0.1:8545`
- A deployed `RollupSettlement` contract
- Foundry `cast` available on `PATH`

## Environment

```bash
export SETTLEMENT_API_URL=http://127.0.0.1:3000
export EVM_RPC_URL=http://127.0.0.1:8545
export ROLLUP_SETTLEMENT_CONTRACT=0x...
export EVM_PRIVATE_KEY=0x...
```

## Commands

Set the sequencer address with the contract owner key:

```bash
cargo run -p settlement-client -- set-sequencer --sequencer 0x...
```

Deposit the sequencer bond with the sequencer key:

```bash
cargo run -p settlement-client -- deposit-bond --value 10ether
```

Submit a built rollup batch:

```bash
cargo run -p settlement-client -- submit-batch --batch-number 0
```

Submit an SP1 proof to `ValidityRollupSettlement`:

```bash
cargo run -p settlement-client -- \
  submit-batch --batch-number 0 --proof batch-proof.json
```

Because validity-only submission finalizes atomically, the client persists both
the submission and finalization milestones from the same verified L1 receipt.

After a successful broadcast, the client extracts the returned L1 transaction
hash, verifies the transaction receipt through JSON-RPC, and persists the
receipt block, gas, chain ID, and raw receipt to the explorer API as the batch's
`submitted` settlement status.

Finalize the batch after the challenge window:

```bash
cargo run -p settlement-client -- finalize-batch --batch-number 0
```

After a successful finalize transaction, the client verifies and persists the
finalize receipt metadata and marks the batch `finalized` in Postgres.

Use `--dry-run` to print the `cast send` command without broadcasting:

```bash
cargo run -p settlement-client -- --dry-run submit-batch --batch-number 0
```

## End-to-end validity settlement test

The ignored Rust integration test creates an isolated Postgres schema, runs the
explorer API, sequences a signed transfer, generates an SP1 Groth16 proof,
deploys the real SP1 verifier and validity settlement contract to Anvil, submits
the proof through this client, and verifies both L1 state and persisted receipt
metadata.

Build the prover once, then run the test explicitly:

```bash
cargo build --release -p validity-prover

TEST_DATABASE_URL=postgresql://postgres:postgres@localhost:5432/zk_rollup \
VALIDITY_PROVER_BIN="$PWD/target/release/validity-prover" \
cargo test -p settlement-client --test validity_settlement_e2e \
  -- --ignored --nocapture
```

The database user must be allowed to create and drop schemas. Anvil, `forge`,
`cast`, and the SP1 proving toolchain must be available on `PATH`.
