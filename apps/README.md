# CLI Commands

This project has four command-line entry points:

```text
wallet-cli      development key generation, signing, and verification
merkle-cli      Merkle root and inclusion proof utilities
chain-cli       local JSON-backed blockchain state machine
chain-indexer   Prisma/Postgres indexer for chain JSON data
```

## Setup

Create a local Prisma environment file:

```bash
cp prisma/.env.example prisma/.env
```

Edit `prisma/.env` with your actual Postgres connection string:

```env
DATABASE_URL="postgresql://postgres:postgres@localhost:5432/zk_rollup"
```

For local Prisma Migrate commands, create the shadow database once:

```bash
docker exec zk-rollup-postgres createdb -U postgres zk_rollup_shadow
```

## Wallet CLI

Create a development keypair:

```bash
cargo run -p wallet-cli -- create
```

Create and save a keypair:

```bash
cargo run -p wallet-cli -- create --output dev-keypair.json
```

Load and validate a saved keypair:

```bash
cargo run -p wallet-cli -- load --key-file dev-keypair.json
```

Derive an address from a saved key file:

```bash
cargo run -p wallet-cli -- address --key-file dev-keypair.json
```

Sign and verify a message:

```bash
cargo run -p wallet-cli -- sign \
  --key-file dev-keypair.json \
  --message "hello rollup"

cargo run -p wallet-cli -- verify \
  --public-key "<base64-public-key>" \
  --message "hello rollup" \
  --signature "<base64-signature>"
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

## Chain CLI

Create a demo chain:

```bash
cargo run -p chain-cli -- --chain-file chain.json init

cargo run -p chain-cli -- --chain-file chain.json account create \
  --balance 100 \
  --key-output alice.json

cargo run -p chain-cli -- --chain-file chain.json account create \
  --balance 0 \
  --key-output bob.json

cargo run -p chain-cli -- --chain-file chain.json genesis finalize
```

Queue a transfer and produce a block:

```bash
cargo run -p chain-cli -- --chain-file chain.json tx send \
  --key-file alice.json \
  --to "<bob-address>" \
  --amount 25

cargo run -p chain-cli -- --chain-file chain.json block produce
```

Inspect and validate the chain:

```bash
cargo run -p chain-cli -- --chain-file chain.json state
cargo run -p chain-cli -- --chain-file chain.json validate
```

## Chain Indexer CLI

With `DATABASE_URL` in `prisma/.env`, apply Prisma migrations and index the
local chain with:

```bash
bunx prisma migrate dev
cargo run -p chain-indexer -- --chain-file chain.json
```

Run it as a long-lived polling worker:

```bash
cargo run -p chain-indexer -- \
  --chain-file chain.json \
  --watch \
  --poll-interval-seconds 5
```

Use `--backup-dir` to save known-good chain snapshots after successful indexing:

```bash
cargo run -p chain-indexer -- \
  --chain-file chain.json \
  --watch \
  --backup-dir backups/chains
```

Use `--init-schema` for local development when you want the Rust indexer to
apply the checked-in SQL migration without Prisma:

```bash
cargo run -p chain-indexer -- --chain-file chain.json --init-schema
```

You can still override the env file per command:

```bash
cargo run -p chain-indexer -- \
  --chain-file chain.json \
  --database-url "postgresql://postgres:postgres@localhost:5432/zk_rollup"
```
