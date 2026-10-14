# Sepolia deposit setup

This deploys the deposit inbox needed to fund an L2 account, using Sepolia test
ETH. It does not deploy a verifier or settlement contract, generate proofs, or
enable withdrawal claims. The current inbox holds deposits and has no function
to release them to the settlement contract: use small test amounts only.

## 1. Prepare tools and wallet

Install [Foundry](https://getfoundry.sh/introduction/installation/) and Node.js.
From the repository root:

```bash
forge --version
cast --version
node --version
git submodule update --init --recursive
cast wallet import rolluplens-sepolia --interactive
```

Enter your test wallet's private key only in the local Foundry prompt, then
choose a keystore password. Do not paste the key into chat, source files, or a
`VITE_` variable. This stores an encrypted keystore; deployment asks you to
unlock it. Fund the corresponding address with Sepolia test ETH for gas.

Set your Sepolia RPC endpoint in the terminal (replace the example):

```bash
export EVM_RPC_URL='https://YOUR-SEPOLIA-RPC-ENDPOINT'
export DEPLOYER_ACCOUNT=rolluplens-sepolia
node scripts/deploy-sepolia-deposit.mjs
```

The helper checks chain ID 11155111 and builds the contracts. This command sends
no transaction. It does not estimate whether your wallet has enough gas funds.

## 2. Deploy and configure

```bash
node scripts/deploy-sepolia-deposit.mjs --broadcast
node scripts/deploy-sepolia-deposit.mjs --configure
```

The broadcast command sends a deployment transaction and saves its output to
`local_testing/sepolia/deposit-deployment.json`. The configure command checks
the successful receipt and contract code, then writes:

- `apps/explorer-ui/.env.local`: `VITE_L1_DEPOSIT_INBOX` (existing other values preserved).
- `local_testing/sepolia/deposit.env`: inbox address, deployment block, and indexer settings.

These files are ignored by Git. Keep the deployment record. If the receipt is
not available yet, retry only `--configure`. If broadcasting reports an error,
inspect your wallet/explorer for a transaction before retrying; it may already
have been sent. The helper refuses another deployment when its record exists.

## 3. Configure the deployed services

Use the generated `deposit.env` values in the appropriate hosting settings:

| Service | Settings |
| --- | --- |
| Explorer API (Render) | Set `L1_DEPOSIT_INBOX` to the deployed address; keep its existing Neon `DATABASE_URL`. Restart/redeploy. |
| Local UI | The helper sets the inbox. Set `VITE_EXPLORER_API_BASE` in `apps/explorer-ui/.env.local` to your deployed API origin and restart Vite. |
| Hosted UI | Set `VITE_L1_DEPOSIT_INBOX` and `VITE_EXPLORER_API_BASE` in the frontend host and rebuild/redeploy. |
| GitHub Actions secrets | `DATABASE_URL` (the same Neon database as the API), `EVM_RPC_URL` (Sepolia), `L1_DEPOSIT_INBOX`. |
| GitHub Actions variables | `L1_DEPOSIT_DEPLOYMENT_BLOCK` from the generated file, `EVM_CONFIRMATIONS=6`, `L1_LOG_CHUNK_BLOCKS=10`, `L1_DEPOSIT_REORG_LOOKBACK=12`. |

Run the repository's **Deposit indexer** workflow manually once configured. It
also runs on a five-minute schedule when scheduled workflows are enabled on the
default branch. The indexer does not need the deployment wallet or private key.
Never put a database connection string into a `VITE_` variable.

For a local indexing tick instead, set `DATABASE_URL` securely in your shell to
the same Neon database as the API, retain the Sepolia `EVM_RPC_URL`, then:

```bash
set -a
source local_testing/sepolia/deposit.env
set +a
cargo run -p operator -- --once --index-deposits-only
```

Do not source local database settings over the Neon URL when testing a hosted
API. The API must already have its genesis initialized. Do not reinitialize an
existing deployment or run test suites against its live database.

## 4. Try the UI

```bash
bun run dev
```

1. Select **Sepolia** in MetaMask; the UI currently does not enforce the L1 network.
2. Connect the wallet and open Deposits. The missing-contract warning should disappear.
3. Keep your own wallet address as the L2 recipient. Try `0.001` Sepolia ETH.
4. Approve the transaction, wait for confirmations, and run/wait for the indexer.
5. Refresh Deposits. Use Operate → Build block if automatic sequencing is off.
6. Refresh Transfers or search your account. The sequenced deposit creates its L2 balance.

Deposits use wei as L2 balance units; `0.001` ETH is `1000000000000000` units.
Use small integer amounts when testing L2 transfers. A wallet balance on Sepolia
alone does not create a rollup account.

## Full settlement is a separate setup

To prove and finalize batches, also deploy the matching official SP1 verifier
and `ValidityRollupSettlement`, using the rollup's **genesis** state root, the
prover's exact program verification key, and this inbox address. Set the
settlement sequencer, configure its address in the UI/operator, and run the full
operator with proving resources. See [contract deployment](README.md#deployment)
and the [operator guide](../apps/operator/README.md).

Do not run the local settlement helper unchanged on Sepolia: it uses local
confirmation defaults and takes the API's current root, which can differ from
the genesis root after sequencing. No complete withdrawal funding path is
established by this deposit-only setup.
