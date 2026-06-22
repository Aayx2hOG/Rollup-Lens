# Applications

Runtime applications are Postgres-backed.

```text
wallet-cli      development key generation, signing, and verification
merkle-cli      Merkle root and inclusion proof utilities
explorer-api    rollup explorer API, transaction pool, and sequencer
explorer-ui     React explorer and sequencer dashboard
```

## Setup

Create a local Prisma environment file:

```bash
cp prisma/.env.example prisma/.env
```

For local development:

```env
DATABASE_URL="postgresql://postgres:postgres@localhost:5432/zk_rollup"
```

Apply migrations:

```bash
bunx prisma migrate dev
```

## Explorer API

```bash
cargo run -p explorer-api -- --bind 127.0.0.1:3000
```

## Explorer UI

```bash
cd apps/explorer-ui
bun install
bun run dev
```

## Checks

```bash
cargo test
cd apps/explorer-ui && bun test && bun run build
```
