# Explorer UI

## Wallet operation

The Send view connects to an injected EVM wallet such as MetaMask or Rabby and
requests an EIP-712 `Transfer` signature. The page never receives private-key
material. The signed L2 transfer contains an Ethereum sender address, recipient,
amount, nonce, and recoverable signature.

The current connector uses the injected `window.ethereum` provider. EIP-6963
wallet discovery and WalletConnect are the next connection-layer milestone.

## Run

On a new database, initialize genesis once with funded Ethereum addresses:

```bash
bun run genesis:init genesis.json
```

Start the UI from `apps/explorer-ui`:

```bash
bun install
bun run dev
```

This starts the Explorer API, waits until it is healthy, and starts Vite. To
point the UI at another API:

```bash
VITE_EXPLORER_API_BASE=http://127.0.0.1:3000 bun run dev
```

## Checks

```bash
bun test
bun run build
```
