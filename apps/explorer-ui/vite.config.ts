import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { defineConfig, loadEnv } from "vite";
import react from "@vitejs/plugin-react";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const operatorEnv = loadOperatorEnv(resolve(root, ".env.operator.local"));

export default defineConfig(({ mode }) => {
  const appEnv = loadEnv(mode, process.cwd(), "VITE_");
  const viteEnv = { ...operatorEnv, ...appEnv, ...process.env };

  return {
    plugins: [react()],
    define: {
      "import.meta.env.VITE_L1_DEPOSIT_INBOX": JSON.stringify(
        viteEnv.VITE_L1_DEPOSIT_INBOX ?? "",
      ),
      "import.meta.env.VITE_ROLLUP_SETTLEMENT_CONTRACT": JSON.stringify(
        viteEnv.VITE_ROLLUP_SETTLEMENT_CONTRACT ?? "",
      ),
    },
    server: {
      port: 5173,
      proxy: {
        "/api": {
          target: "http://127.0.0.1:3000",
          changeOrigin: true,
          rewrite: (path) => path.replace(/^\/api/, ""),
        },
      },
    },
  };
});

function loadOperatorEnv(filePath: string) {
  const env: Record<string, string> = {};
  if (!existsSync(filePath)) return env;

  for (const line of readFileSync(filePath, "utf8").split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;

    const separator = trimmed.indexOf("=");
    if (separator === -1) continue;

    const key = trimmed.slice(0, separator).trim();
    if (!key.startsWith("VITE_")) continue;

    env[key] = unquoteEnvValue(trimmed.slice(separator + 1).trim());
  }
  return env;
}

function unquoteEnvValue(value: string) {
  if (
    (value.startsWith("'") && value.endsWith("'"))
    || (value.startsWith('"') && value.endsWith('"'))
  ) {
    return value.slice(1, -1);
  }
  return value;
}
