# ZK Rollup Portfolio

This repository is a Rust-first blockchain infrastructure portfolio. It starts
with wallet and Merkle primitives, then grows into indexing, rollup simulation,
analytics APIs, and a dashboard.

## Workspace

```text
crates/
  wallet-core/      Reusable signing, verification, and address logic
  merkle-core/      Deterministic Merkle roots and inclusion proofs
  chain-core/       Accounts, transactions, blocks, state roots, validation
contracts/
  RollupSettlement.sol  Optimistic L1 settlement rules for rollup batches
apps/
  wallet-cli/       CLI wrapper around wallet-core
  merkle-cli/       CLI wrapper around merkle-core
  chain-cli/        JSON-backed local blockchain CLI
  chain-indexer/    Prisma/Postgres indexer for chain JSON data
  explorer-api/     Read-only HTTP explorer over Phase 3 chain JSON
docs/
  system-design.md
  phase-1-wallet.md
  phase-2-merkle.md
  phase-3-chain.md
  phase-4-explorer.md
```

CLI commands are collected in [`apps/README.md`](apps/README.md).

## System Design

The current and planned architecture is documented in
[`docs/system-design.md`](docs/system-design.md). It includes a Mermaid
flowchart and SVG diagram covering the write path, chain storage, indexing,
read API, cache, explorer UI, rollup batches, and observability.

## Phase 1: Wallet CLI

Create a development keypair:

```bash
cargo run -p wallet-cli -- create
```

Create and save a plaintext development keypair:

```bash
cargo run -p wallet-cli -- create --output dev-keypair.json
```

Load and validate a saved development keypair:

```bash
cargo run -p wallet-cli -- load --key-file dev-keypair.json
```

Import a keypair from an existing secret key:

```bash
cargo run -p wallet-cli -- import \
  --secret-key "<base64-secret-key>" \
  --output imported-keypair.json
```

Derive a public key from a secret key:

```bash
cargo run -p wallet-cli -- public-key --secret-key "<base64-secret-key>"
```

Derive a public key from a saved key file:

```bash
cargo run -p wallet-cli -- public-key --key-file dev-keypair.json
```

Derive an address from a public key:

```bash
cargo run -p wallet-cli -- address --public-key "<base64-public-key>"
```

Derive an address from a saved key file:

```bash
cargo run -p wallet-cli -- address --key-file dev-keypair.json
```

Sign a message:

```bash
cargo run -p wallet-cli -- sign \
  --secret-key "<base64-secret-key>" \
  --message "hello rollup"
```

Sign a message with a saved key file:

```bash
cargo run -p wallet-cli -- sign \
  --key-file dev-keypair.json \
  --message "hello rollup"
```

Verify a signature:

```bash
cargo run -p wallet-cli -- verify \
  --public-key "<base64-public-key>" \
  --message "hello rollup" \
  --signature "<base64-signature>"
```

Run tests:

```bash
cargo test
```

## Phase 2: Merkle Tree CLI

Create a JSON leaf file:

```json
[
  { "account": "alice", "balance": 100 },
  { "account": "bob", "balance": 50 },
  { "account": "carol", "balance": 25 }
]
```

Compute a Merkle root:

```bash
cargo run -p merkle-cli -- root leaves.json
```

Generate an inclusion proof:

```bash
cargo run -p merkle-cli -- prove leaves.json 1 > proof.json
```

Verify the proof:

```bash
cargo run -p merkle-cli -- verify \
  --leaf '{"account":"bob","balance":50}' \
  --proof proof.json \
  --root "<root-hex>"
```

Merkle proofs are emitted as JSON so later API services and dashboards can
consume them directly.

## Phase 3: Minimal Blockchain CLI

Initialize a local JSON chain:

```bash
cargo run -p chain-cli -- --chain-file chain.json init
```

Create funded genesis accounts before producing blocks:

```bash
cargo run -p chain-cli -- --chain-file chain.json account create \
  --balance 100 \
  --key-output alice.json

cargo run -p chain-cli -- --chain-file chain.json account create \
  --balance 0 \
  --key-output bob.json
```

Finalize the genesis block:

```bash
cargo run -p chain-cli -- --chain-file chain.json genesis finalize
```

Queue a signed transfer:

```bash
cargo run -p chain-cli -- --chain-file chain.json tx send \
  --key-file alice.json \
  --to "<bob-address>" \
  --amount 25
```

Produce a block from pending transactions:

```bash
cargo run -p chain-cli -- --chain-file chain.json block produce
```

Inspect state and validate the full chain from genesis:

```bash
cargo run -p chain-cli -- --chain-file chain.json state
cargo run -p chain-cli -- --chain-file chain.json validate
```

Generate a rollup batch commitment suitable for the settlement contract:

```bash
cargo run -p chain-cli -- --chain-file chain.json rollup batch \
  --batch-number 0 \
  --start-height 1 \
  --end-height 1
```

Block `0` is the genesis block. It has the null parent hash, no transactions,
and a deterministic Merkle state root over the initial accounts. Later blocks
include parent hashes, signed transactions, state roots, and block hashes.
Validation replays the chain from the explicit genesis block.

## Rollup Settlement Security

[`contracts/RollupSettlement.sol`](contracts/RollupSettlement.sol) adds an
optimistic settlement layer around the Rust state machine. The contract requires
a bonded sequencer, state-root continuity, transaction/data commitments, a
single pending batch, challenge bonds, a challenge window before finality, and
forced-withdrawal requests for censorship resistance. The Rust CLI can export
the roots and hashes that map directly to `submitBatch`.

## Phase 4: Blockchain Explorer API

Start a read-only explorer API over a Phase 3 chain file:

```bash
cargo run -p explorer-api -- --json-mode --chain-file chain.json --bind 0.0.0.0:3000
```

Available endpoints:

```text
GET /health
GET /
GET /chain/summary
GET /blocks
GET /blocks/:height
GET /blocks/hash/:hash
GET /accounts/:address
GET /transactions/:hash
GET /search/:query
```

`/search/:query` resolves block hashes, block heights, account addresses, and
transaction hashes. Transaction hashes are SHA-256 commitments over the
serialized signed transfer.

## Phase 5: Prisma/Postgres Indexer

The Postgres schema is defined in [`prisma/schema.prisma`](prisma/schema.prisma),
with the matching SQL migration in
[`prisma/migrations/0001_init/migration.sql`](prisma/migrations/0001_init/migration.sql).

Start a local Postgres container for development:

```bash
docker run --name zk-rollup-postgres \
  -e POSTGRES_USER=postgres \
  -e POSTGRES_PASSWORD=postgres \
  -e POSTGRES_DB=zk_rollup \
  -p 5432:5432 \
  -d postgres:16
```

If the container already exists, start it instead:

```bash
docker start zk-rollup-postgres
```

Create the local shadow database used by Prisma Migrate:

```bash
docker exec zk-rollup-postgres createdb -U postgres zk_rollup_shadow
```

Create a local Prisma env file:

```bash
cp prisma/.env.example prisma/.env
```

For the local container above, `prisma/.env` should contain:

```env
DATABASE_URL="postgresql://postgres:postgres@localhost:5432/zk_rollup"
```

Apply Prisma migrations, then index a validated local chain file:

```bash
bunx prisma migrate dev
cargo run -p chain-indexer -- --chain-file chain.json
```

Run the indexer as a cron-style polling worker while `chain.json` changes:

```bash
cargo run -p chain-indexer -- \
  --chain-file chain.json \
  --watch \
  --poll-interval-seconds 5
```

Persist known-good chain snapshots after successful indexing:

```bash
cargo run -p chain-indexer -- \
  --chain-file chain.json \
  --watch \
  --backup-dir backups/chains
```

`--init-schema` is a Rust-only local shortcut when you are not using Prisma
Migrate:

```bash
cargo run -p chain-indexer -- --chain-file chain.json --init-schema
```

In Postgres mode, the explorer API keeps a short in-memory cache for hot read
endpoints and exposes recent persistent errors through `GET /errors` and
`GET /errors/latest`.

## Explorer UI

The React explorer frontend lives in [`apps/explorer-ui`](apps/explorer-ui).
It uses Bun, Vite, Tailwind, TanStack Query, Framer Motion, and lucide icons.

```bash
cd apps/explorer-ui
bun install
bun run dev
```

During development, Vite proxies `/api/*` to the explorer API on
`http://127.0.0.1:3000`.
