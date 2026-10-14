# Automated Rollup Operator

The operator closes the loop between the transaction pool and
`ValidityRollupSettlement`:

1. ask the explorer API to sequence pending transfers
2. claim exactly the batch number expected by the L1 contract
3. export its persisted validity witness
4. generate an SP1 Groth16 artifact
5. submit the proof through `settlement-client --no-persist`
6. scan `BatchFinalized` logs through the L1 `safe` tag and verify every root against Postgres
7. persist safe/finalized metadata from the authoritative L1 event

Batch finality is driven by the execution client's `safe` and `finalized` block
tags, not by a hand-picked confirmation depth. The operator stores every
observed batch finalization event with its L1 block hash, transaction hash, and
log index, rescans a trailing window, and orphans non-finalized events if the
canonical block hash changes.

When `L1_DEPOSIT_INBOX` is configured, the same lifecycle tick also scans
`DepositAccepted` logs from `L1DepositInbox` and persists canonical L1 deposits
into `l1_deposits`. Deposit scans are idempotent across restarts and use a small
lookback window so recent reorgs can replace older rows by marking the competing
deposit as `orphaned`.

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
export SETTLEMENT_API_URL=http://127.0.0.1:3000
export EVM_RPC_URL=http://127.0.0.1:8545
export ROLLUP_SETTLEMENT_CONTRACT=0x...
export EVM_PRIVATE_KEY=0x...
export EVM_DEPLOYMENT_BLOCK=0
export EVM_CONFIRMATIONS=6
export L1_BATCH_REORG_LOOKBACK=64
export L1_DEPOSIT_INBOX=0x...
export L1_DEPOSIT_DEPLOYMENT_BLOCK=0
export L1_DEPOSIT_REORG_LOOKBACK=12
export VALIDITY_PROVER_BIN="$PWD/target/release/validity-prover"
export SETTLEMENT_CLIENT_BIN="$PWD/target/release/settlement-client"

./target/release/operator
```

Run exactly one lifecycle tick and exit with:

```bash
./target/release/operator --once
```

`--once` performs the same indexing, sequencing, proving, and submission path
as the daemon. It skips PostgreSQL `LISTEN` and the polling loop, which makes it
suitable for deterministic integration tests, one-shot jobs, and debugging.
Any tick failure is returned as a non-zero process exit.

For local Anvil testing, the repository can deploy and configure the complete
settlement environment:

```bash
mkdir -p local_testing
anvil --host 127.0.0.1 --port 8545 --state local_testing/anvil-state.json
export EVM_PRIVATE_KEY='0x<private-key printed by anvil>'
export DATABASE_URL='postgresql://postgres:postgres@localhost:5432/zk_rollup'
bun run settlement:deploy:local
set -a
source .env.operator.local
set +a
cargo run -p operator
```

This writes the ignored, owner-readable `.env.operator.local` file used by
the local operator command. See the root README for prerequisites and generated
values. `bun run settlement:deploy:local` also reads `local_testing/state.env`
when present, checks that `L1_DEPOSIT_INBOX` has code on `EVM_RPC_URL`, and
writes `L1_DEPOSIT_DEPLOYMENT_BLOCK` plus `VITE_L1_DEPOSIT_INBOX` into
`.env.operator.local`.

## Deposit index-only mode

For hosted demos, RollupLens can index `L1DepositInbox` evidence without running
the sequencer, prover, or settlement client:

```bash
set -a
source .env.operator.local
set +a
cargo run -p operator -- --once --index-deposits-only
```

This mode requires `DATABASE_URL`, `EVM_RPC_URL`, and `L1_DEPOSIT_INBOX`. Set
`L1_DEPOSIT_DEPLOYMENT_BLOCK` to the inbox deployment block when available; if
it is omitted, the operator starts at the current confirmed L1 head and only
tracks new deposits going forward. It does not require `EVM_PRIVATE_KEY` or
`ROLLUP_SETTLEMENT_CONTRACT`, and it is the path used by the scheduled GitHub
Actions deposit indexer.

The scheduled `Deposit indexer` workflow requires these repository secrets:
`DATABASE_URL` or `NEON_DATABASE_URL`, `EVM_RPC_URL`, and `L1_DEPOSIT_INBOX`.
Keep `L1_DEPOSIT_DEPLOYMENT_BLOCK` set to the actual inbox deployment block
when possible so a hosted run does not accidentally scan from genesis on a
public RPC.

Use `DATABASE_URL` for local Docker Postgres and `NEON_DATABASE_URL` for the
deployed database. If the deployed URL uses a transaction pooler, also set
`DIRECT_DATABASE_URL` to a direct endpoint because PostgreSQL `LISTEN` requires
a persistent session. When both `DATABASE_URL` and `NEON_DATABASE_URL` are set,
the operator deliberately prefers `DATABASE_URL`.

For Anvil, use `EVM_CONFIRMATIONS=0` for deposit indexing. In production, set
`EVM_DEPLOYMENT_BLOCK` to the settlement contract's deployment block and
`L1_DEPOSIT_DEPLOYMENT_BLOCK` to the deposit inbox deployment block. The L1 RPC
used by the operator must support `eth_getBlockByNumber` with `safe` and
`finalized` tags. Omit `L1_DEPOSIT_INBOX` when a deployment does not have an
inbox yet.

Before indexing deposits, verify that the configured inbox exists on the active
RPC:

```bash
cast code "$L1_DEPOSIT_INBOX" --rpc-url "$EVM_RPC_URL"
```

The result must be non-empty bytecode, not `0x`. If local Anvil was reset,
delete stale addresses from `local_testing/state.env`, deploy a new
`L1DepositInbox`, and rerun `bun run settlement:deploy:local`.

Proof artifacts are retained under `operator-artifacts/` by default. Move the
directory with `OPERATOR_ARTIFACT_DIR`; it should not be committed.

## Trust and failure model

The submission client uses `--no-persist`: a successful broadcast is only an
attempted transition. A batch becomes finalized in Postgres after the indexer
observes the matching event at or below the RPC `finalized` block, fetches its
successful receipt, and matches its batch number, roots, commitments, and
transaction count to local data.

The batch checkpoint advances only after every event in a scanned range is
validated and applied. Restarting resumes from that checkpoint, but each tick
rescans the last `L1_BATCH_REORG_LOOKBACK` blocks before the checkpoint. Events
below the `safe` tag are projected as safe settlement evidence. A batch is marked
`finalized` in Postgres only after the event's block is at or below the RPC
`finalized` tag and its stored block hash still matches canonical history.
Non-finalized events whose block hash changes, or whose log is reported with
`removed: true`, are marked `orphaned` and their projected batch settlement
metadata is unwound.

Deposit indexing deliberately rescans the last `L1_DEPOSIT_REORG_LOOKBACK`
blocks before the checkpoint on every tick. Exact repeats only update
confirmations. A same-ID deposit observed with a different block hash,
transaction hash, or log index orphans the older canonical row before inserting
the replacement, preserving one non-orphaned deposit per inbox and deposit ID.

The explorer's legacy settlement-update endpoint is disabled in the production
binary unless `SETTLEMENT_UPDATE_TOKEN` is set. Manual `settlement-client`
usage must provide the same variable. The automated operator does not use that
endpoint.

## Deterministic integration test

`tests/operator_once.rs` runs the real operator binary against an isolated
PostgreSQL schema, an in-process API/JSON-RPC server, and executable fake prover
and settlement-client processes. It verifies:

```text
queued -> proving -> proved -> broadcast -> finalized
```

With the local Docker database running, use the existing `DATABASE_URL`:

```bash
source prisma/.env
cargo test -p operator --test operator_once -- --ignored --nocapture
```

The test creates and drops its own uniquely named schema. It is marked ignored
so a normal `cargo test` never connects to the database implicitly. It never
falls back to `NEON_DATABASE_URL`.

## Automatic deposits and L2 transfers (without proving)

Use `--process-l2-only` for the hosted demo. Every poll (five seconds by default)
it indexes canonical, confirmed L1 deposits, then calls the API sequencer to
include pending deposits, transfers, and withdrawals. Users do not need to run
an indexer or click Build block. This mode does not prove, settle, or claim
withdrawals; batches remain built until a full settlement operator processes them.

It requires no signing key, settlement contract, SP1 installation, or persistent
Postgres LISTEN connection. It reuses the existing durable indexer checkpoints,
reorg handling, and transactional sequencing. Run one processing worker per
rollup, and disable the scheduled deposit-indexer workflow for that rollup to
avoid concurrent indexers. Do not run this mode alongside the full operator.

### Deployment

`render.yaml` now declares a `rollup-lens-processor` background worker and leaves
sequencing to that worker instead of the API's auto-sequence loop. Apply the
updated Blueprint or create a Background Worker manually (pushing code to an
existing standalone API service does not create the worker):

```text
Build: cargo build --release -p operator
Start: ./target/release/operator --process-l2-only
```

The configured worker compute plan is paid. No hosting resources are provisioned
by editing the YAML. See [Render worker documentation](https://render.com/docs/background-workers)
and [compute plans](https://render.com/docs/compute-plans).

Set these values once in the hosting dashboard:

| Variable | Where / value |
| --- | --- |
| `DATABASE_URL` | API and worker: the same Neon database containing the initialized rollup. |
| `L1_DEPOSIT_INBOX` | API and worker: the same deployed inbox as the UI. Restart the API after changing it. |
| `OPERATOR_API_TOKEN` | API and worker: the same secret token; never put this in the frontend. |
| `SETTLEMENT_API_URL` | Worker: deployed API base URL, without `/api`. |
| `EVM_RPC_URL` | Worker: RPC URL for the chain containing the inbox, e.g. Sepolia. |
| `L1_DEPOSIT_DEPLOYMENT_BLOCK` | Worker: actual inbox deployment block, required to avoid skipping earlier deposits. |
| `EVM_CONFIRMATIONS` | Worker: `6` by default. |
| `OPERATOR_POLL_SECONDS` | Worker: `5` by default. |
| `L1_LOG_CHUNK_BLOCKS` | Worker: `10` by default; tune for your RPC provider. |

Apply committed database migrations separately before starting the services.
Keep the API available: a sleeping/free API can delay sequencing during cold
starts. Indexing old blocks can take multiple RPC calls; the poll interval is
not a guarantee that a deposit is credited within five seconds. Six L1
confirmations must also elapse.

The worker logs startup and failures, records tick errors in the existing
`error_events` table, and retries on the next poll. An indexer failure stops
that tick before sequencing. HTTP calls time out after 60 seconds. API failure
after indexing leaves confirmed deposits in Postgres for the next attempt.

### Local verification

Export the variables above using a local/test environment, then start the API
and worker in separate terminals:

```bash
cargo run -p explorer-api -- --bind 127.0.0.1:3000
```

```bash
export SETTLEMENT_API_URL=http://127.0.0.1:3000
cargo run -p operator -- --process-l2-only
```

For one processing pass use:

```bash
cargo run -p operator -- --process-l2-only --once
```

Test in the UI by making one small testnet deposit, waiting for confirmations,
and watching it become executed without Build block. Submit an L2 transfer;
it should also be included automatically. Stop/restart the worker and verify
pending activity resumes. Stop the API briefly: the worker should log failures,
then recover once it returns. These manual checks require a configured chain,
inbox, database, and funded test wallet.

Automated checks:

```bash
cargo test -p operator --bin operator
```

The lightweight-mode regression test uses a local mock RPC/API (no database or
wallet required). It verifies sequencing, token forwarding, repeated polls,
index-only behavior, and no sequencing after an RPC failure. Existing ignored
Postgres integration suites remain separate; this unit check does not verify
live deposit crediting.
