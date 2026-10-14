import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outputDir = resolve(root, "local_testing/sepolia");
const recordPath = resolve(outputDir, "deposit-deployment.json");
const rpc = process.env.EVM_RPC_URL;
const broadcast = process.argv.includes("--broadcast");
const configure = process.argv.includes("--configure");

function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, encoding: "utf8", stdio: ["inherit", "pipe", "inherit"] });
  if (result.error || result.status !== 0) throw new Error(`${command} failed. Check the tool installation and configuration.`);
  return result.stdout.trim();
}
function updateEnv(path, values) {
  let content = existsSync(path) ? readFileSync(path, "utf8") : "";
  for (const [key, value] of Object.entries(values)) {
    const line = `${key}=${value}`;
    const pattern = new RegExp(`^${key}=.*$`, "gm");
    content = pattern.test(content) ? content.replace(pattern, () => line) : `${content.trimEnd()}\n${line}\n`;
  }
  writeFileSync(path, content, { mode: 0o600 });
}

try {
  if (!rpc) throw new Error("Set EVM_RPC_URL to your Sepolia RPC endpoint first.");
  const chainId = run("cast", ["chain-id", "--rpc-url", rpc]);
  if (chainId !== "11155111") throw new Error(`Expected Sepolia chain 11155111; received ${chainId}. Nothing deployed.`);
  mkdirSync(outputDir, { recursive: true, mode: 0o700 });
  if (configure) {
    if (broadcast) throw new Error("Use --configure or --broadcast, not both.");
    const deployment = JSON.parse(readFileSync(recordPath, "utf8"));
    const address = deployment.deployedTo ?? deployment.deployed_to;
    const hash = deployment.transactionHash ?? deployment.transaction_hash;
    if (!/^0x[0-9a-fA-F]{40}$/.test(address ?? "") || !/^0x[0-9a-fA-F]{64}$/.test(hash ?? "")) {
      throw new Error("Deployment record is missing its address or transaction hash.");
    }
    const receipt = JSON.parse(run("cast", ["receipt", hash, "--rpc-url", rpc, "--json"]));
    if (BigInt(receipt.status) !== 1n || receipt.contractAddress?.toLowerCase() !== address.toLowerCase()) {
      throw new Error("Receipt does not confirm a successful deployment at this address.");
    }
    if (run("cast", ["code", address, "--rpc-url", rpc]) === "0x") throw new Error("No contract code at deployment address.");
    const block = BigInt(receipt.blockNumber).toString();
    updateEnv(resolve(root, "apps/explorer-ui/.env.local"), { VITE_L1_DEPOSIT_INBOX: address });
    updateEnv(resolve(outputDir, "deposit.env"), {
      L1_DEPOSIT_INBOX: address,
      L1_DEPOSIT_DEPLOYMENT_BLOCK: block,
      EVM_CONFIRMATIONS: "6",
      L1_DEPOSIT_REORG_LOOKBACK: "12",
      L1_LOG_CHUNK_BLOCKS: "10",
    });
    console.log(`UI configured with ${address}.\nIndexer settings: local_testing/sepolia/deposit.env\nRestart Vite and configure the API/indexer with this same inbox.`);
  } else {
    if (existsSync(recordPath)) throw new Error("Deployment record already exists. Use --configure to configure it; do not redeploy.");
    console.log(run("forge", ["build"]));
    if (!broadcast) {
      console.log("Sepolia RPC and contract build checked. No transaction sent.\nNext: set DEPLOYER_ACCOUNT to your Foundry keystore name, then rerun with --broadcast.");
    } else {
      const account = process.env.DEPLOYER_ACCOUNT;
      if (!account) throw new Error("Set DEPLOYER_ACCOUNT to the encrypted Foundry keystore name.");
      console.log("Deploying L1DepositInbox on Sepolia using the selected keystore. Unlock it when prompted.");
      const output = run("forge", ["create", "contracts/L1DepositInbox.sol:L1DepositInbox", "--rpc-url", rpc, "--account", account, "--broadcast", "--json"]);
      // Preserve the output before parsing, so a configuration failure never triggers a redeploy.
      writeFileSync(recordPath, output + "\n", { mode: 0o600, flag: "wx" });
      console.log(`Deployment output saved to ${recordPath}.\nRun again with --configure to verify the receipt and write environment settings.`);
    }
  }
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
