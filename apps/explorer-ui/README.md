# Explorer UI

React/Vite explorer dashboard for the local zk-rollup chain.

## Run

Start the API in JSON mode:

```bash
cargo run -p explorer-api -- --json-mode --chain-file chain.json --bind 127.0.0.1:3000
```

Start the UI:

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
