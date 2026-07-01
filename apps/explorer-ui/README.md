# Explorer UI

React/Vite explorer dashboard for the local zk-rollup chain.

## Run

On a new database, initialize genesis once from a real account allocation as
described in the repository README:

```bash
bun run genesis:init genesis.json
```

Start the Postgres-backed API from the repository root:

```bash
bun run api:dev
```

In a second terminal, start the UI from `apps/explorer-ui`:

```bash
bun install
bun run dev
```

The Vite dev server proxies `/api/*` to `http://127.0.0.1:3000`.

To point the UI at a different API without the dev proxy:

```bash
VITE_EXPLORER_API_BASE=http://127.0.0.1:3000 bun run dev
```

## Checks

```bash
bun test
bun run build
```
