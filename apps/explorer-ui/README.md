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
cargo run -p explorer-api -- --initialize-genesis genesis.json
```

Start the UI from `apps/explorer-ui`:

```bash
bun install
bun run dev
```

This starts Vite only. Start the Explorer API separately before opening the UI:

```bash
cargo run -p explorer-api -- --bind 127.0.0.1:3000
```

To point the UI at another API:

```bash
VITE_EXPLORER_API_BASE=http://127.0.0.1:3000 bun run dev
```

To submit validity proofs from the Settlement tab, also expose the deployed L1
contracts to Vite:

```bash
VITE_L1_DEPOSIT_INBOX=0x... VITE_ROLLUP_SETTLEMENT_CONTRACT=0x... bun run dev
```

For local development, keep `VITE_L1_DEPOSIT_INBOX` aligned with
`L1_DEPOSIT_INBOX`, and run Anvil with:

```bash
mkdir -p local_testing
anvil --host 127.0.0.1 --port 8545 --state local_testing/anvil-state.json
```

## Checks

```bash
bun test
bun run build
```
