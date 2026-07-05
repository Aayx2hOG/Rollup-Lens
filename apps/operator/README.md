# Automated Rollup Operator

The operator closes the loop between the transaction pool and
`ValidityRollupSettlement`:

1. ask the explorer API to sequence pending transfers
2. claim exactly the batch number expected by the L1 contract
3. export its persisted validity witness
4. generate an SP1 Groth16 artifact
5. submit the proof through `settlement-client --no-persist`
6. scan confirmed `BatchFinalized` logs and verify every root against Postgres
7. persist finality and receipt metadata from the authoritative L1 event

Batch claims and lifecycle transitions are stored in Postgres. A crashed
proving job can be reclaimed after two hours, failed jobs retry up to
`OPERATOR_MAX_ATTEMPTS`, and broadcast batches are not resubmitted while the
indexer waits for confirmation.

The explorer API publishes a PostgreSQL `NOTIFY` on
`rollup_transaction_pending` after a submitted transaction commits. The
operator uses `LISTEN` to wake immediately and start a lifecycle tick.
Notifications are only wake-up signals: transaction and batch rows remain the
durable source of truth, and `OPERATOR_POLL_SECONDS` polling remains enabled as
a recovery path for disconnects, restarts, or missed notifications.

## Setup

Apply migrations, then build the three binaries:

```bash
bun run db:migrate
cargo build --release -p validity-prover -p settlement-client -p operator
```

Configure and run:

```bash
export DATABASE_URL=postgresql://postgres:postgres@localhost:5432/zk_rollup
export DIRECT_DATABASE_URL="$DATABASE_URL"
export SETTLEMENT_API_URL=http://127.0.0.1:3000
export EVM_RPC_URL=http://127.0.0.1:8545
export ROLLUP_SETTLEMENT_CONTRACT=0x...
export EVM_PRIVATE_KEY=0x...
export EVM_DEPLOYMENT_BLOCK=0
export EVM_CONFIRMATIONS=6
export VALIDITY_PROVER_BIN="$PWD/target/release/validity-prover"
export SETTLEMENT_CLIENT_BIN="$PWD/target/release/settlement-client"

./target/release/operator
```

For local Anvil testing, the repository can deploy and configure the complete
settlement environment:

```bash
export EVM_PRIVATE_KEY='0x<private-key printed by anvil>'
export DATABASE_URL='postgresql://postgres:postgres@localhost:5432/zk_rollup'
bun run settlement:deploy:local
bun run operator:local
```

This writes the ignored, owner-readable `.env.operator.local` file used by
`operator:local`. See the root README for prerequisites and generated values.

When using Neon, set `DATABASE_URL` to the pooled URL for ordinary application
queries and `DIRECT_DATABASE_URL` to the non-pooler URL. PostgreSQL
`LISTEN` requires a persistent session and must not use Neon's transaction
pooler.

For Anvil, use `EVM_CONFIRMATIONS=0`. In production, set
`EVM_DEPLOYMENT_BLOCK` to the contract's deployment block.

Proof artifacts are retained under `operator-artifacts/` by default. Move the
directory with `OPERATOR_ARTIFACT_DIR`; it should not be committed.

## Trust and failure model

The submission client uses `--no-persist`: a successful broadcast is only an
attempted transition. A batch becomes finalized in Postgres after the indexer
observes a sufficiently confirmed event, fetches its successful receipt, and
matches its batch number, roots, commitments, and transaction count to local
data.

The checkpoint advances only after every event in a scanned range is validated
and applied. Restarting resumes from that checkpoint. Confirmation depth
protects against ordinary shallow reorgs; after a reorg deeper than
`EVM_CONFIRMATIONS`, rewind the checkpoint manually and reconcile affected
batch rows.

The explorer's legacy settlement-update endpoint is disabled in the production
binary unless `SETTLEMENT_UPDATE_TOKEN` is set. Manual `settlement-client`
usage must provide the same variable. The automated operator does not use that
endpoint.
