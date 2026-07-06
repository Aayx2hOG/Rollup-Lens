import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const maxAttempts = 3;
const useNeon = process.argv.includes("--neon");
const resetEip712 = process.argv.includes("--reset-eip712");
const migrationEnv = { ...Bun.env };

if (resetEip712 && !useNeon) {
  console.error("--reset-eip712 requires --neon.");
  process.exit(1);
}

if (useNeon) {
  const neonDatabaseUrl =
    Bun.env.NEON_DATABASE_URL?.trim() ??
    databaseUrlFromPrismaEnv("NEON_DATABASE_URL");

  if (!neonDatabaseUrl) {
    console.error(
      "Set NEON_DATABASE_URL or add it to prisma/.env before running db:migrate:neon.",
    );
    process.exit(1);
  }

  // prisma.config.ts prefers DIRECT_DATABASE_URL. Override both variables so
  // a local direct URL cannot accidentally redirect a Neon migration.
  migrationEnv.DATABASE_URL = neonDatabaseUrl;
  migrationEnv.DIRECT_DATABASE_URL = neonDatabaseUrl;
  console.log("Deploying committed migrations to NEON_DATABASE_URL.");
}

if (resetEip712) {
  console.warn(
    "Archiving existing rollup state to legacy_ed25519_archive before the EIP-712 reset.",
  );
  await runPrisma([
    "migrate",
    "resolve",
    "--rolled-back",
    "0009_eip712_ethereum_accounts",
  ]);
  await executeSql(`
    CREATE SCHEMA legacy_ed25519_archive;

    CREATE TABLE legacy_ed25519_archive.accounts AS TABLE public.accounts;
    CREATE TABLE legacy_ed25519_archive.blocks AS TABLE public.blocks;
    CREATE TABLE legacy_ed25519_archive.transactions AS TABLE public.transactions;
    CREATE TABLE legacy_ed25519_archive.transaction_pool AS TABLE public.transaction_pool;
    CREATE TABLE legacy_ed25519_archive.rollup_batches AS TABLE public.rollup_batches;
    CREATE TABLE legacy_ed25519_archive.rollup_batch_transactions AS TABLE public.rollup_batch_transactions;
    CREATE TABLE legacy_ed25519_archive.indexer_checkpoints AS TABLE public.indexer_checkpoints;
    CREATE TABLE legacy_ed25519_archive.l1_indexer_checkpoints AS TABLE public.l1_indexer_checkpoints;
    CREATE TABLE legacy_ed25519_archive.error_events AS TABLE public.error_events;

    TRUNCATE TABLE
      public.rollup_batch_transactions,
      public.rollup_batches,
      public.transaction_pool,
      public.transactions,
      public.blocks,
      public.accounts,
      public.indexer_checkpoints,
      public.l1_indexer_checkpoints,
      public.error_events
    RESTART IDENTITY;
  `);
  console.log("Legacy state archived and active rollup state cleared.");
}

for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
  const child = Bun.spawn(["bunx", "prisma", "migrate", "deploy"], {
    cwd: import.meta.dir + "/..",
    env: migrationEnv,
    stdout: "inherit",
    stderr: "pipe",
  });
  const stderr = await new Response(child.stderr).text();
  const exitCode = await child.exited;

  if (stderr) {
    await Bun.write(Bun.stderr, stderr);
  }
  if (exitCode === 0) {
    process.exitCode = 0;
    break;
  }

  const lockTimedOut = stderr.includes("P1002") || stderr.includes("advisory lock");
  if (!lockTimedOut || attempt === maxAttempts) {
    process.exitCode = exitCode;
    break;
  }

  const delaySeconds = attempt * 12;
  console.warn(
    `Migration lock is busy. Retrying in ${delaySeconds} seconds (${attempt + 1}/${maxAttempts})...`,
  );
  await Bun.sleep(delaySeconds * 1000);
}

async function runPrisma(args) {
  const child = Bun.spawn(["bunx", "prisma", ...args], {
    cwd: import.meta.dir + "/..",
    env: migrationEnv,
    stdout: "inherit",
    stderr: "inherit",
  });
  const exitCode = await child.exited;
  if (exitCode !== 0) {
    process.exit(exitCode);
  }
}

async function executeSql(sql) {
  const child = Bun.spawn(["bunx", "prisma", "db", "execute", "--stdin"], {
    cwd: import.meta.dir + "/..",
    env: migrationEnv,
    stdin: "pipe",
    stdout: "inherit",
    stderr: "inherit",
  });
  child.stdin.write(sql);
  child.stdin.end();
  const exitCode = await child.exited;
  if (exitCode !== 0) {
    process.exit(exitCode);
  }
}

function databaseUrlFromPrismaEnv(key) {
  const envPath = resolve(import.meta.dir, "../prisma/.env");
  let contents;
  try {
    contents = readFileSync(envPath, "utf8");
  } catch {
    return undefined;
  }

  const line = contents
    .split(/\r?\n/)
    .find((entry) => entry.trim().startsWith(`${key}=`));
  if (!line) {
    return undefined;
  }

  return line
    .slice(line.indexOf("=") + 1)
    .trim()
    .replace(/^['"]|['"]$/g, "");
}
