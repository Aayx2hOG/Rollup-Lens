# ZK Rollup Portfolio

Rust-first rollup infrastructure project with a Postgres-backed execution,
indexing, explorer, and sequencing path.

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
                    rollup batches, transaction pool, and error events
```

`chain-core` remains a pure deterministic execution library. Runtime state lives
in Postgres, not in local JSON files.

## System Design

The production-shaped topology has four application/data server boundaries,
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
   RPC to read contract state, submit proofs, and index confirmed events.
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
[attachment and data-flow diagram](docs/system-design-flowchart.svg).

## Setup

Start Postgres:

```bash
bun run db:up
```

Create `prisma/.env`:

```bash
cp prisma/.env.example prisma/.env
```

For the local container above:

```env
DATABASE_URL="postgresql://postgres:postgres@localhost:5432/zk_rollup"
DIRECT_DATABASE_URL="postgresql://postgres:postgres@localhost:5432/zk_rollup"
```

With Neon, use the pooled connection string for `DATABASE_URL` and the
non-pooler connection string from Neon's Connect dialog for
`DIRECT_DATABASE_URL`. Prisma migrations prefer the direct URL; the running API
continues to use the pooled URL.

`bun run db:up` and `bun run db:down` are only for the optional local Docker
database. They are not needed when `prisma/.env` points to Neon. Check the
configured database without changing it:

```bash
bun run db:status
```

Do not run integration tests against the deployed application's production
branch. Create a separate Neon test branch, then pass its connection string
only to the test process:

```bash
TEST_DATABASE_URL='postgresql://...neon.tech/neondb?sslmode=require&channel_binding=require' \
cargo test -p settlement-client --test validity_settlement_e2e \
  sequences_and_finalizes_a_batch_with_mock_verifier -- --ignored --nocapture
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
bun run genesis:init
```

Or invoke the binary directly:

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
bun run api:dev
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
GET  /rollup/batches
POST /rollup/batches
GET  /rollup/batches/:batch_number
GET  /rollup/batches/:batch_number/settlement-calldata
GET  /rollup/batches/:batch_number/validity-witness
GET  /errors
GET  /errors/latest
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
from confirmed `BatchFinalized` events whose commitments match Postgres.

See [`apps/operator/README.md`](apps/operator/README.md) for configuration and
failure semantics.

### One-command local settlement deployment

For a reproducible local validity-settlement environment, start Anvil and the
explorer API, then provide one funded Anvil key and the application database:

```bash
export EVM_PRIVATE_KEY='0x<private-key printed by anvil>'
export DATABASE_URL='postgresql://postgres:postgres@localhost:5432/zk_rollup'
bun run settlement:deploy:local
bun run operator:local
```

The deployment command builds the required release binaries, reads the
initialized genesis root from the API, derives the validity guest's SP1 program
key, deploys the matching SP1 Groth16 verifier and
`ValidityRollupSettlement`, configures the key's address as sequencer, and
writes `.env.operator.local`. That generated file contains the private key and
database URL, is ignored by Git, and is created with owner-only permissions.

The API must be available at `http://127.0.0.1:3000` and Anvil at
`http://127.0.0.1:8545` by default. Override those locations with
`SETTLEMENT_API_URL` and `EVM_RPC_URL`. Re-running the deployment intentionally
creates a fresh settlement chain and replaces the local operator configuration.

## Explorer UI

Run the UI:

```bash
cd apps/explorer-ui
bun install
bun run dev
```

From the repository root, the same UI command is also available as:

```bash
bun run ui:dev
```

For day-to-day development after the database is configured and migrations are
applied, start the complete application:

```bash
bun run dev
```

The UI launcher starts the API automatically, waits for its health endpoint,
and then starts Vite. `Ctrl+C` stops both. The same integrated command is
available inside `apps/explorer-ui` as `bun run dev`, so a separate backend
terminal is not required.

Vite proxies `/api/*` to `http://127.0.0.1:3000` during development. The UI
includes overview, block, account, rollup batch, sequencer, and API health
views.

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
part of the explicit genesis allocation (or receive funds through a future
deposit path) before it can submit transfers.

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

```bash
cargo test
cd apps/explorer-ui && bun test && bun run build
```
