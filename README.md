# ZK Rollup Portfolio

Rust-first rollup infrastructure project with a Postgres-backed execution,
indexing, explorer, and sequencing path.

## Workspace

```text
crates/
  wallet-core/      Signing, verification, and address logic
  merkle-core/      Deterministic Merkle roots and inclusion proofs
  chain-core/       Accounts, transfers, blocks, state roots, validation
contracts/
  RollupSettlement.sol  Optimistic L1 settlement rules for rollup batches
apps/
  wallet-cli/       Development wallet utilities
  merkle-cli/       Merkle proof utilities
  explorer-api/     Postgres-backed rollup API and sequencer
  explorer-ui/      React explorer and sequencer UI
prisma/
  schema.prisma     Postgres schema for blocks, transactions, accounts,
                    rollup batches, transaction pool, and error events
```

`chain-core` remains a pure deterministic execution library. Runtime state lives
in Postgres, not in local JSON files.

## Setup

Start Postgres:

```bash
docker run --name zk-rollup-postgres \
  -e POSTGRES_USER=postgres \
  -e POSTGRES_PASSWORD=postgres \
  -e POSTGRES_DB=zk_rollup \
  -p 5432:5432 \
  -d postgres:16
```

Create `prisma/.env`:

```bash
cp prisma/.env.example prisma/.env
```

For the local container above:

```env
DATABASE_URL="postgresql://postgres:postgres@localhost:5432/zk_rollup"
```

Apply migrations:

```bash
bunx prisma migrate dev
```

## Explorer API

Run the Postgres-backed API:

```bash
cargo run -p explorer-api -- --bind 0.0.0.0:3000
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
GET  /errors
GET  /errors/latest
GET  /search/:query
```

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

## Explorer UI

Run the UI:

```bash
cd apps/explorer-ui
bun install
bun run dev
```

Vite proxies `/api/*` to `http://127.0.0.1:3000` during development. The UI
includes overview, block, account, rollup batch, sequencer, and API health
views.

## Wallet CLI

Create a development keypair:

```bash
cargo run -p wallet-cli -- create
```

Sign a message:

```bash
cargo run -p wallet-cli -- sign \
  --key-file dev-keypair.json \
  --message "hello rollup"
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
