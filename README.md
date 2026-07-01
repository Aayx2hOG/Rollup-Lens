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
prisma/
  schema.prisma     Postgres schema for blocks, transactions, accounts,
                    rollup batches, transaction pool, and error events
```

`chain-core` remains a pure deterministic execution library. Runtime state lives
in Postgres, not in local JSON files.

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
```

Apply migrations:

```bash
bun run db:migrate
```

## Initialize Genesis

Genesis is a one-time state allocation, not demo data. Create or import the
wallets that should control the allocated balances:

```bash
cargo run -p wallet-cli -- create --output alice-keypair.json
cargo run -p wallet-cli -- address --key-file alice-keypair.json
```

Create `genesis.json` using the printed address:

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
bun run genesis:init genesis.json
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

From the repository root, the same UI command is also available as:

```bash
bun run ui:dev
```

For day-to-day development after Postgres is running and migrations are applied,
start both the API and UI together:

```bash
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
