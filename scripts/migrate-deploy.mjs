import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const maxAttempts = 3;
const useNeon = process.argv.includes("--neon");
const migrationEnv = { ...Bun.env };
let fallbackMigrationEnv;

if (useNeon) {
  const neonDatabaseUrl =
    Bun.env.NEON_DATABASE_URL?.trim() ??
    databaseUrlFromPrismaEnv("NEON_DATABASE_URL");
  const neonDirectDatabaseUrl =
    Bun.env.NEON_DIRECT_DATABASE_URL?.trim() ??
    databaseUrlFromPrismaEnv("NEON_DIRECT_DATABASE_URL") ??
    directNeonUrlFromPooler(neonDatabaseUrl);

  if (!neonDatabaseUrl) {
    console.error(
      "Set NEON_DATABASE_URL or add it to prisma/.env before running db:migrate:neon.",
    );
    process.exit(1);
  }
  if (!neonDirectDatabaseUrl) {
    console.error(
      "Set NEON_DIRECT_DATABASE_URL or use a NEON_DATABASE_URL host containing -pooler.",
    );
    process.exit(1);
  }

  // Prisma migrations should use Neon's direct endpoint, not the pooler. Keep
  // DATABASE_URL on the pooler for app-runtime parity, and override
  // DIRECT_DATABASE_URL so prisma.config.ts does not select a local database.
  migrationEnv.DATABASE_URL = neonDatabaseUrl;
  migrationEnv.DIRECT_DATABASE_URL = neonDirectDatabaseUrl;
  if (neonDirectDatabaseUrl !== neonDatabaseUrl) {
    fallbackMigrationEnv = {
      ...migrationEnv,
      DIRECT_DATABASE_URL: neonDatabaseUrl,
    };
  }
  console.log("Deploying committed migrations to Neon direct database URL.");
}

const result = await deployWithRetries(migrationEnv);
if (
  result.exitCode !== 0 &&
  fallbackMigrationEnv &&
  result.stderr.includes("P1001")
) {
  console.warn(
    "Neon direct endpoint was unreachable. Retrying migration through NEON_DATABASE_URL.",
  );
  const fallbackResult = await deployWithRetries(fallbackMigrationEnv);
  process.exitCode = fallbackResult.exitCode;
} else {
  process.exitCode = result.exitCode;
}

async function deployWithRetries(env) {
  let lastExitCode = 1;
  let lastStderr = "";
  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    const result = await runPrismaDeploy(env);
    lastExitCode = result.exitCode;
    lastStderr = result.stderr;

    if (result.exitCode === 0) {
      return result;
    }

    const lockTimedOut =
      result.stderr.includes("P1002") || result.stderr.includes("advisory lock");
    if (!lockTimedOut || attempt === maxAttempts) {
      return result;
    }

    const delaySeconds = attempt * 12;
    console.warn(
      `Migration lock is busy. Retrying in ${delaySeconds} seconds (${attempt + 1}/${maxAttempts})...`,
    );
    await Bun.sleep(delaySeconds * 1000);
  }

  return { exitCode: lastExitCode, stderr: lastStderr };
}

async function runPrismaDeploy(env) {
  const child = Bun.spawn(["bunx", "prisma", "migrate", "deploy"], {
    cwd: import.meta.dir + "/..",
    env,
    stdout: "inherit",
    stderr: "pipe",
  });
  const stderr = await new Response(child.stderr).text();
  const exitCode = await child.exited;

  if (stderr) {
    await Bun.write(Bun.stderr, stderr);
  }
  return { exitCode, stderr };
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

function directNeonUrlFromPooler(databaseUrl) {
  if (!databaseUrl) {
    return undefined;
  }
  let url;
  try {
    url = new URL(databaseUrl);
  } catch {
    return undefined;
  }
  if (!url.hostname.includes("-pooler.")) {
    return databaseUrl;
  }
  url.hostname = url.hostname.replace("-pooler.", ".");
  return url.toString();
}
