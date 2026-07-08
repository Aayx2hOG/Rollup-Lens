import { spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const uiRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = path.resolve(uiRoot, "../..");
const apiHealthUrl = "http://127.0.0.1:3000/health";
const children = new Set();
let shuttingDown = false;

loadLocalEnv(path.resolve(repositoryRoot, "local_testing/state.env"));
loadLocalEnv(path.resolve(repositoryRoot, ".env.operator.local"));
if (!process.env.VITE_L1_DEPOSIT_INBOX && process.env.L1_DEPOSIT_INBOX) {
  process.env.VITE_L1_DEPOSIT_INBOX = process.env.L1_DEPOSIT_INBOX;
}
if (!process.env.VITE_ROLLUP_SETTLEMENT_CONTRACT && process.env.ROLLUP_SETTLEMENT_CONTRACT) {
  process.env.VITE_ROLLUP_SETTLEMENT_CONTRACT = process.env.ROLLUP_SETTLEMENT_CONTRACT;
}

function start(command, args, cwd) {
  const child = spawn(command, args, {
    cwd,
    stdio: "inherit",
    env: process.env,
    detached: process.platform !== "win32",
  });
  children.add(child);
  child.once("exit", () => children.delete(child));
  return child;
}

function loadLocalEnv(filePath) {
  if (!existsSync(filePath)) return;
  const content = readFileSync(filePath, "utf8");
  for (const line of content.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const separator = trimmed.indexOf("=");
    if (separator === -1) continue;
    const key = trimmed.slice(0, separator).trim();
    if (!/^[A-Z0-9_]+$/.test(key) || process.env[key]) continue;
    process.env[key] = unquoteEnvValue(trimmed.slice(separator + 1).trim());
  }
}

function unquoteEnvValue(value) {
  if (
    (value.startsWith("'") && value.endsWith("'"))
    || (value.startsWith('"') && value.endsWith('"'))
  ) {
    return value.slice(1, -1);
  }
  return value;
}

function stop(child) {
  if (!child?.pid || child.exitCode !== null) return;
  try {
    if (process.platform === "win32") child.kill("SIGTERM");
    else process.kill(-child.pid, "SIGTERM");
  } catch {
    // The process may already have exited.
  }
}

function shutdown(exitCode = 0) {
  if (shuttingDown) return;
  shuttingDown = true;
  for (const child of children) stop(child);
  setTimeout(() => process.exit(exitCode), 50);
}

async function apiIsReady() {
  try {
    const response = await fetch(apiHealthUrl, {
      signal: AbortSignal.timeout(1_000),
    });
    return response.ok;
  } catch {
    return false;
  }
}

async function waitForApi(apiProcess) {
  const deadline = Date.now() + 180_000;
  while (Date.now() < deadline) {
    if (apiProcess.exitCode !== null) {
      throw new Error(`Explorer API exited with code ${apiProcess.exitCode}.`);
    }
    if (await apiIsReady()) return;
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error("Timed out waiting for the Explorer API to become ready.");
}

process.once("SIGINT", () => shutdown(130));
process.once("SIGTERM", () => shutdown(143));

let apiProcess;
try {
  if (process.env.VITE_EXPLORER_API_BASE) {
    console.log("Using the API configured by VITE_EXPLORER_API_BASE.");
  } else {
    if (await apiIsReady()) {
      throw new Error(
        "Explorer API is already running on 127.0.0.1:3000. Stop that process, then run `bun run dev` again.",
      );
    }
    console.log("Starting the Explorer API...");
    apiProcess = start(
      "cargo",
      ["run", "-p", "explorer-api", "--", "--bind", "127.0.0.1:3000", "--auto-sequence"],
      repositoryRoot,
    );
    await waitForApi(apiProcess);
    console.log("Explorer API is ready.");
  }

  const viteProcess = start(
    "bunx",
    ["vite", "--host", "0.0.0.0"],
    uiRoot,
  );
  viteProcess.once("exit", (code, signal) => {
    if (!shuttingDown) shutdown(signal ? 1 : (code ?? 0));
  });
  apiProcess?.once("exit", (code) => {
    if (!shuttingDown) {
      console.error(`Explorer API stopped unexpectedly (code ${code ?? "unknown"}).`);
      shutdown(code ?? 1);
    }
  });
} catch (error) {
  console.error(error instanceof Error ? error.message : error);
  shutdown(1);
}
