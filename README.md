# RollupLens

RollupLens is a Rust-based rollup lifecycle explorer. It demonstrates how
wallet-signed transactions, L1 deposits, L2 sequencing, batch commitments, and
optional proof/settlement flows fit together. The hosted demo is educational
and testnet-focused; full proving requires heavier compute.

This repository is an educational infrastructure prototype, not a public
production L2, a real bridge for user funds, or a trustless live network. Use
local chains and testnets only.

## Workspace

```text
crates/
  wallet-core/      Signing, verification, and address logic
  merkle-core/      Ordered and sparse Merkle roots and proofs
  chain-core/       Accounts, transfers, blocks, state roots, validation
  validity-core/    Shared batch validity statement and witness verification
contracts/
  RollupSettlement.sol  Optimistic L1 settlement rules for rollup batches
  ValidityRollupSettlement.sol  Minimal SP1 validity-only settlement
apps/
  wallet-cli/       Development wallet utilities
  merkle-cli/       Merkle proof utilities
  explorer-api/     Postgres-backed rollup API and sequencer
  explorer-ui/      React explorer and sequencer UI
  validity-prover/  SP1 batch execution, proving, and local verification
  operator/         Automated sequencing, proving, submission, and L1 indexing
prisma/
  schema.prisma     Postgres schema for blocks, transactions, accounts,
                    rollup batches, pools, L1 evidence, and operator state
```

`chain-core` remains a pure deterministic execution library. Runtime state lives
in Postgres, not in local JSON files.

## System Design

The deployment-shaped topology has four application/data server boundaries,
plus the external L1 RPC/network:

1. **Web server or CDN** serves the compiled explorer UI. An EVM wallet signs
   EIP-712 transfers; private keys never reach the UI or a server.
2. **Explorer API server** exposes the Axum HTTP API. It validates signed
   transfers, persists the transaction pool, runs deterministic sequencing, and
   exports immutable validity witnesses and settlement data.
3. **PostgreSQL server** is the durable source of truth for accounts, blocks,
   transactions, batches, operator claims, L1 receipts, checkpoints, and error
   events. The API publishes `NOTIFY rollup_transaction_pending` after a pending
   transaction commits.
4. **Operator server** holds the long-running operator and launches
   `validity-prover` and `settlement-client` as child processes. It connects
   directly to PostgreSQL for claims, lifecycle state, and `LISTEN`; calls the
   Explorer API for sequencing and immutable witness exports; and calls the EVM
   RPC to read contract state, submit proofs, and index safe/finalized events.
5. **External EVM RPC and L1 contracts** provide the authoritative settlement
   state. `ValidityRollupSettlement` verifies each SP1 proof through its
   configured verifier and emits `BatchFinalized`.

`NOTIFY/LISTEN` is a low-latency wake-up path, not a queue. PostgreSQL rows are
durable, and the operator's periodic poll runs the same lifecycle tick after a
disconnect, restart, or missed notification. This makes delivery effectively
at-least-once at the wake-up layer while database claims and on-chain batch
numbers keep processing idempotent.

The smallest practical deployment can co-locate the web server, API, and
operator on one machine, but they remain separate processes and should scale or
restart independently. PostgreSQL must provide a persistent direct connection
for `LISTEN`; a transaction-pooler-only connection is insufficient.

See the expanded
[attachment/data-flow diagram](docs/system-design-flowchart.svg).

## Setup

Create `prisma/.env`:

```bash
cp prisma/.env.example prisma/.env
```

Start the local Postgres service:

```bash
docker compose up -d postgres
```

The Docker service is exposed on `localhost:5434`:

```env
DATABASE_URL="postgresql://postgres:postgres@localhost:5434/zk_rollup"
DIRECT_DATABASE_URL="postgresql://postgres:postgres@localhost:5434/zk_rollup"
```

With Neon, use the pooled connection string for `DATABASE_URL` and the
non-pooler connection string from Neon's Connect dialog for
`DIRECT_DATABASE_URL`. Prisma migrations prefer the direct URL; the running API
continues to use the pooled URL.

Check the configured database without changing it:

```bash
bun run db:status
```

## Demo and Developer Modes

### Hosted Demo Mode

Use this mode for a public educational demo. The UI runs on Vercel, the API can
run on Render, Postgres can run on Neon, and the deposit indexer can run from
GitHub Actions every five minutes. This mode prioritizes readable lifecycle
evidence over full proof generation.

Required pieces:

```text
Vercel: explorer UI
Render: explorer API
Neon: Postgres
GitHub Actions or another scheduler: lightweight deposit indexer
```

Configure the scheduled indexer with `DATABASE_URL` or `NEON_DATABASE_URL`,
`EVM_RPC_URL`, and `L1_DEPOSIT_INBOX` secrets. Set
`L1_DEPOSIT_DEPLOYMENT_BLOCK` to the inbox deployment block when available; if
it is omitted, the indexer starts at the current confirmed L1 head and only
tracks new deposits going forward. It scans deposit logs only; it does not
sequence, prove, submit settlement transactions, or need `EVM_PRIVATE_KEY`.

### Sepolia deposits

To use Sepolia test ETH with the hosted API/Neon database, follow the
[Sepolia deployment guide](contracts/SEPOLIA.md). The helper deploys the deposit
inbox and writes its address into the local UI environment.

### Local Developer Mode

Use this mode to run the DB/API/UI, connect a wallet, inspect deposits, submit
L2 transfers, sequence batches, and run mock operator tests. The basic explorer
demo does not require the full prover.

Run local Anvil with a persisted state file so deployed L1 contracts survive
restarts:

```bash
mkdir -p local_testing
anvil --host 127.0.0.1 --port 8545 --state local_testing/anvil-state.json
```

Keep local contract and service values in `local_testing/state.env`. The local
deployment helper copies those values into `.env.operator.local`; use that file
when starting services that need L1 deposit configuration.

```bash
bun run db:migrate
set -a
source .env.operator.local
set +a
cargo run -p explorer-api -- --bind 127.0.0.1:3000
```

Start the UI in a second terminal:

```bash
bun run --cwd apps/explorer-ui dev
```

After making an L1 deposit, run one lightweight indexing tick, refresh the UI,
and use **Operate -> Build block** to sequence confirmed deposits into an L2
batch:

```bash
set -a
source .env.operator.local
set +a
cargo run -p operator -- --once --index-deposits-only
```

This deposit-only operator mode is safe for low-memory local demos. It indexes
`DepositAccepted` logs only; it does not run the SP1 prover, submit settlement
transactions, or finalize batches on L1.

The API must be started with the same `L1_DEPOSIT_INBOX` that appears on the
deposit rows. If the API starts without `.env.operator.local`, it falls back to
the zero address and **Operate -> Build block** will report that there are no
confirmed deposits to sequence. Stale zero-address dummy deposits should be
orphaned or removed from the local database.

Before depositing from the UI, verify that the configured inbox exists on the
current Anvil state:

```bash
set -a
source local_testing/state.env
set +a
cast code "$L1_DEPOSIT_INBOX" --rpc-url "$EVM_RPC_URL"
```

The result must be non-empty bytecode, not `0x`.

### Full Proof Mode

Use this mode only when you need end-to-end settlement evidence. It requires an
Anvil or testnet RPC, deployed contracts, release binaries, the full operator,
and SP1 Groth16 proving resources.

The full proof path can be RAM-heavy because it invokes
`validity-prover groth16`. See [`apps/operator/README.md`](apps/operator/README.md)
and [`apps/settlement-client/README.md`](apps/settlement-client/README.md) for
the full settlement commands.

Do not run integration tests against the deployed application's production
branch. Create a separate Neon test branch, then pass its connection string
only to the test process:

```bash
TEST_DATABASE_URL='postgresql://...neon.tech/neondb?sslmode=require&channel_binding=require' \
cargo test -p settlement-client --test validity_settlement_e2e \
  deposit_transfer_proof_and_settlement_lifecycle -- --ignored --nocapture
```

The test role must be allowed to create and drop schemas.

Apply migrations:

```bash
bun run db:migrate
```

Migration `0009_eip712_ethereum_accounts` changes account identity from the
original Ed25519 format to Ethereum addresses. Existing addresses and
signatures cannot be converted without their private keys, so use a fresh local
database or Neon branch, apply migrations, and initialize a new genesis
allocation with Ethereum addresses.

## Initialize Genesis

Genesis is a one-time state allocation, not demo data. Create or import the
wallets that should control the allocated balances:

```bash
cargo run -p wallet-cli -- create --output alice-evm-keypair.json
cargo run -p wallet-cli -- address --key-file alice-evm-keypair.json
```

Create `genesis.json` using the printed Ethereum address. After the EIP-712
migration, old Ed25519/base64 addresses are invalid here.

```json
{
  "accounts": [
    {
      "address": "<address printed by wallet-cli>",
      "balance": 1000000,
      "nonce": 0
    }
  ]
}
```

Initialize the database:

```bash
cargo run -p explorer-api -- \
  --initialize-genesis genesis.json
```

The command calculates the sparse account-state root and genesis block through
`chain-core`, writes the block and all accounts atomically, prints the resulting
state root and block hash, and exits. It refuses to run when blocks or accounts
already exist.

Keep keypair files outside source control. Genesis contains only public
addresses, balances, and nonces; private keys never belong in the allocation
file or database.

## Explorer API

Run the Postgres-backed API:

```bash
cargo run -p explorer-api -- --bind 127.0.0.1:3000 --auto-sequence
```

You can also pass the database URL explicitly:

```bash
cargo run -p explorer-api -- \
  --database-url "postgresql://postgres:postgres@localhost:5432/zk_rollup" \
  --bind 0.0.0.0:3000
```

Core endpoints:

```text
GET  /health
GET  /ready
GET  /
GET  /chain/summary
GET  /blocks
GET  /blocks/:height
GET  /blocks/hash/:hash
GET  /accounts/:address
GET  /transactions/:hash
GET  /rollup/transactions
POST /rollup/transactions
GET  /rollup/withdrawals
POST /rollup/withdrawals
GET  /rollup/batches
POST /rollup/batches
GET  /rollup/batches/:batch_number
GET  /rollup/batches/:batch_number/settlement-calldata
GET  /rollup/batches/:batch_number/validity-witness
GET  /rollup/batches/:batch_number/withdrawals/:withdrawal_hash/proof
GET  /errors
GET  /errors/latest
GET  /jobs
GET  /alerts
GET  /search/:query
```

`/health` reports application state stored by the explorer. `/ready` actively
checks Postgres and returns HTTP 503 with a safe diagnostic when the database
is unavailable.

`POST /rollup/transactions` accepts a signed transfer and stores it in the
durable transaction pool after validating it against current account state plus
already-pending transactions.

`POST /rollup/batches` runs the local sequencer. It locks pending pool rows,
validates them deterministically, writes the next block, updates account
snapshots, marks pool rows as included or failed, and rebuilds the deterministic
rollup batch commitment.

`GET /rollup/batches/:batch_number/settlement-calldata` exports the built batch
in the argument order expected by `RollupSettlement.submitBatch`, including
Solidity-ready `0x`-prefixed `bytes32` roots.

The sequencer also persists the batch's private pre-state and current blocks
atomically. `GET /rollup/batches/:batch_number/validity-witness` exports that
input for `validity-prover`; a prover does not need to reconstruct historical
account state from the latest snapshot.

Each sequencing operation appends an immutable batch with the next batch
number. Existing or finalized batch commitments are never rebuilt when later
transactions arrive.

## Automated Operator

After deploying `ValidityRollupSettlement`, the operator runs the complete
batch lifecycle continuously. It sequences pending transfers, proves only the
batch currently expected by the L1 contract, submits it, and derives finality
from `BatchFinalized` events whose commitments match Postgres and whose L1
blocks have reached the RPC `safe` or `finalized` tags.

See [`apps/operator/README.md`](apps/operator/README.md) for configuration and
failure semantics.

## Explorer UI

From the repository root, start the Explorer UI:

```bash
bun run dev
```

This starts Vite only. Start the Explorer API separately first:

```bash
set -a
source .env.operator.local
set +a
cargo run -p explorer-api -- --bind 127.0.0.1:3000
```

You can run the same UI command from `apps/explorer-ui` when you are already
working in that folder:

```bash
cd apps/explorer-ui
bun install
bun run dev
```

Vite proxies `/api/*` to `http://127.0.0.1:3000` during development. The UI
includes an expanded getting-started guide on Overview and collapsible help on
each page. Use **Lifecycle** to browse blocks and transactions without a wallet,
**Deposits** to add test funds, **Transfers** to send or request withdrawals,
and **Batches & settlement** to follow L1 evidence. **Operate** and **Prove**
contain the advanced sequencing and proof workflows.

See the [UI usage guide and browser checklist](apps/explorer-ui/README.md) for
step-by-step instructions and expected results.

## Production deployment

The repository root contains `render.yaml` for a Render web service. It builds
the Rust API, checks `/ready`, and starts the API on Render's assigned port.
Apply committed Prisma migrations as a separate deployment step so overlapping
service starts cannot contend for Prisma's advisory lock.

Set `DATABASE_URL` as a secret in Render using the complete Neon connection
string. Do not add quotes around the value. For an existing manually configured
service, use:

```text
Build: cargo build --release -p explorer-api
Migration: bun run db:migrate:deploy
Start: ./target/release/explorer-api --bind 0.0.0.0:$PORT --auto-sequence
Health check: /ready
```

When the UI is hosted separately, set its build-time API origin:

```env
VITE_EXPLORER_API_BASE=https://your-api.onrender.com
```

The Send view connects to an injected EVM wallet and asks it to sign an EIP-712 L2
transfer. The private key remains inside the wallet; the explorer receives only
the Ethereum address and signature.

Connecting a wallet does not allocate funds. Its address must already be
part of the explicit genesis allocation or receive a confirmed, sequenced L1
deposit before it can submit transfers.

## Development wallet CLI

Create a development Ethereum keypair:

```bash
cargo run -p wallet-cli -- create
```

Sign an EIP-712 rollup transfer:

```bash
cargo run -p wallet-cli -- sign-transfer \
  --key-file dev-keypair.json \
  --to 0x2222222222222222222222222222222222222222 \
  --amount 10 \
  --nonce 0
```

## Merkle CLI

Compute a Merkle root:

```bash
cargo run -p merkle-cli -- root leaves.json
```

Generate and verify an inclusion proof:

```bash
cargo run -p merkle-cli -- prove leaves.json 1 > proof.json

cargo run -p merkle-cli -- verify \
  --leaf '{"account":"bob","balance":50}' \
  --proof proof.json \
  --root "<root-hex>"
```

## Contracts

[`contracts/RollupSettlement.sol`](contracts/RollupSettlement.sol) models an
optimistic settlement layer with bonded sequencing, state-root continuity,
challenge windows, and forced-withdrawal requests.

Run the contract tests with Foundry:

```bash
forge test
```

## Checks

Run commands below from the repository root. Install Rust/Cargo, Bun, and
Foundry (`forge`, `anvil`, `cast`); database tests also require PostgreSQL.

### UI and standard automated checks

```bash
bun install --cwd apps/explorer-ui
bun run --cwd apps/explorer-ui test
bun run --cwd apps/explorer-ui build
cargo test
forge test
```

The UI tests cover formatting and wallet/signature interactions; the build
checks TypeScript and creates the production bundle. They do not replace the
[manual browser checklist](apps/explorer-ui/README.md#manual-browser-checklist),
which covers navigation, mobile layout, keyboard access, search, wallet flows,
and error recovery. `cargo test` skips ignored integration tests, and API
Postgres tests can return early when no database URL is set. A green default
run alone does not verify the full lifecycle.

### Database and operator integration checks

Use a disposable local database or separate Neon test branch, with a role that
can create/drop schemas. These suites create isolated schemas. Do not point
tests at the deployed application's production database.

```bash
docker compose up -d postgres
TEST_DATABASE_URL=postgresql://postgres:postgres@localhost:5432/zk_rollup \
cargo test -p explorer-api --test routes -- --nocapture

DATABASE_URL=postgresql://postgres:postgres@localhost:5432/zk_rollup \
cargo test -p operator --test operator_once -- --ignored --nocapture
```

The API suite checks routes against Postgres; inspect output for skipped tests.
The operator suite uses fake prover/settlement processes and a mock RPC to
verify orchestration through finalization without running SP1.

### End-to-end settlement

With Postgres and Foundry available, run the fast lifecycle test (real local
contracts and a deterministic verifier mock):

```bash
TEST_DATABASE_URL=postgresql://postgres:postgres@localhost:5432/zk_rollup \
cargo test -p settlement-client --test validity_settlement_e2e \
  deposit_transfer_proof_and_settlement_lifecycle -- --ignored --nocapture
```

For real cryptographic proof coverage, install the SP1 toolchain and provide
sufficient proving resources, then run:

```bash
cargo build --release -p validity-prover
TEST_DATABASE_URL=postgresql://postgres:postgres@localhost:5432/zk_rollup \
VALIDITY_PROVER_BIN="$PWD/target/release/validity-prover" \
cargo test -p settlement-client --test validity_settlement_e2e \
  proves_sequences_and_finalizes_a_batch_on_anvil -- --ignored --nocapture
```

Both tiers check deposits, transfer execution, balances/nonces, batch settlement,
and receipt metadata. See the [settlement test guide](apps/settlement-client/README.md#end-to-end-validity-settlement-tests)
for prerequisites and the [operator guide](apps/operator/README.md) for service
configuration. Complete the UI browser checklist separately, including the
withdrawal claim flow when full settlement is configured.
