# Applications

Runtime applications are Postgres-backed.

```text
wallet-cli      development Ethereum keys and EIP-712 transfer signing
merkle-cli      Merkle root and inclusion proof utilities
explorer-api    rollup explorer API, transaction pool, and sequencer
explorer-ui     React explorer and sequencer dashboard
validity-prover SP1 batch execution, proving, and local verification
settlement-client validity-proof submission to the EVM settlement contract
operator        automated sequencing, proving, submission, and L1 indexing
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
