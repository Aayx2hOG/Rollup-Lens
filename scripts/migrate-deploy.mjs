import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const maxAttempts = 3;
const useNeon = process.argv.includes("--neon");
const migrationEnv = { ...Bun.env };

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
