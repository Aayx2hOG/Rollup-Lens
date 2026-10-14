# Explorer UI

## Find your way around

The overview opens a **Start here · How to use RollupLens** guide. Every other
page has a collapsible **How to use this page** guide with direct navigation
buttons. Navigation wraps on small screens; on desktop it stays visible as you
scroll. **Batches & settlement** is one destination.

| Page | What to do |
| --- | --- |
| Overview | Check connection status, recent activity, and search for a block height, hash, or account address. |
| Lifecycle | Select a block, inspect a transaction, and open sender/recipient accounts. No wallet required. |
| Deposits | Connect a browser EVM wallet, enter an L2 recipient and ETH amount, and approve an L1 deposit. Wait for indexing, confirmation, and sequencing. |
| Transfers | With a funded L2 account, enter a recipient and amount and sign a transfer. Use the exit form to request a withdrawal. |
| Batches & settlement | Select a batch and follow its commitments, status, and L1 receipt evidence. |
| Prove | Export a witness, generate a proof externally, and load the matching artifact to submit from your wallet. |
| Operate | Inspect pending activity and errors. Choose **Build block** to sequence manually when automation is not running. |

L1 is the connected Ethereum chain; L2 is this rollup. Connecting a wallet does
not fund it. Use a genesis allocation or a confirmed, sequenced deposit. Deposit
amounts are entered in ETH; transfers use the L2 balance units displayed by the
app. Use only local chains or testnets. L1 transactions need test ETH for gas.

A signed transfer enters the pending pool. Sequencing includes it in a block
and batch; **built** does not mean finalized on L1. Proof generation, submission,
and indexing L1 finality are separate steps. A withdrawal needs a finalized
batch before it can be claimed on L1 in a separate wallet transaction.

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

## Manual browser checklist

Run the API and UI above, then open http://127.0.0.1:5173. For wallet flows,
configure the local deposit/settlement contracts using the [root setup guide](../../README.md#local-developer-mode).

| Check | Steps and expected result |
| --- | --- |
| Navigation and help | Open every navigation button and guide link. Exactly one navigation item is active. Expand/collapse help with mouse and with Tab + Enter/Space. Overview help starts expanded; other pages start collapsed. |
| Responsive layout | Try 375px, 768px, and desktop widths plus 200% zoom. All navigation destinations remain reachable without horizontal navigation scrolling. Desktop navigation stays visible while scrolling. |
| Read-only exploration | Without a wallet, select a block in Lifecycle, inspect a transaction and its accounts, and select a batch. Existing records show their details; empty lists explain that no data is available. |
| Search | Search for a known block height, account address, and transaction hash. Check the corresponding details. Try an unknown value and confirm an error is shown. |
| Wallet | Connect and disconnect a browser wallet. Reject a connection request and confirm an error is shown. Without a wallet extension, confirm connection gives a useful error. |
| Deposit | Send a small local/testnet deposit to your address. Run the deposit indexer, wait for the configured confirmations, refresh, and build a block. Confirm the deposit is included and the L2 balance increases. |
| Transfer | Send to a second address from a funded L2 wallet. Sign, check the pending queue, then build a block (or wait for automation). Confirm inclusion, sender/recipient balances, and sender nonce. Reject a signature and verify nothing is queued. |
| Withdrawal | Request an exit from a funded L2 wallet. Sequence it, prove/settle the batch, and wait for indexed finality. Claim on L1; verify the receipt and claimed state after indexing/refresh. Requires full settlement setup. |
| Proof and settlement | Select a built batch in Prove, export its witness, generate a matching proof, load its artifact, and submit. Confirm indexed L1 evidence before expecting finalized status. Invalid artifact JSON should show an error. |
| Error and recovery | Stop the API and refresh: expect a connection/retry or stale-data notice. Restart it and retry. Stop Postgres to check database-unavailable handling, then restart and retry. |

Use **Refresh data** after indexing or other external actions. If a deposit stays
pending, check inbox configuration and confirmations. If a transfer stays
pending, check that the sequencer is running or use Operate → Build block.
Proof generation happens outside the browser and requires additional tooling.

For automated checks across the API, operator, and contracts, see the
[root testing guide](../../README.md#checks).
