# Explorer UI

## Wallet operation

The Operate view supports `wallet-cli` JSON key files and browser-generated
keypairs with the same Ed25519 format. It constructs the domain-separated
`chain-core` transfer payload, signs locally, and submits only the signed
transfer to `POST /rollup/transactions`.

Keys are not written to browser storage or sent to the explorer API. A newly
created keypair is exported immediately and remains usable in memory until the
page is reloaded or the wallet is locked.

React/Vite explorer dashboard for the local zk-rollup chain.

## Run

On a new database, initialize genesis once from a real account allocation as
described in the repository README:

```bash
bun run genesis:init genesis.json
```

Start the UI from `apps/explorer-ui`:

```bash
bun install
bun run dev
```

This single command starts the Explorer API, waits until it is healthy, and
then starts Vite. Stopping it also stops both processes. If an API is already
healthy on `127.0.0.1:3000`, it is reused and left running when Vite stops.
Vite proxies `/api/*` to that API.

To point the UI at a different API, the launcher skips the local API:

```bash
VITE_EXPLORER_API_BASE=http://127.0.0.1:3000 bun run dev
```

The database must still be reachable through `DATABASE_URL` or `prisma/.env`,
and its migrations and one-time genesis initialization must already be applied.

## Checks

```bash
bun test
bun run build
```
