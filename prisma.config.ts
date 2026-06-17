import { readFileSync } from "node:fs";
import { resolve } from "node:path";

export default {
  schema: "prisma/schema.prisma",
  migrations: {
    path: "prisma/migrations",
  },
  datasource: {
    url: databaseUrl(),
    shadowDatabaseUrl: shadowDatabaseUrl(),
  },
};

function databaseUrl(): string {
  const fromEnv = process.env.DATABASE_URL?.trim();
  if (fromEnv) {
    return fromEnv;
  }

  const fromFile = databaseUrlFromPrismaEnv();
  if (fromFile) {
    return fromFile;
  }

  throw new Error("Set DATABASE_URL or add DATABASE_URL to prisma/.env");
}

function shadowDatabaseUrl(): string {
  const fromEnv = process.env.SHADOW_DATABASE_URL?.trim();
  if (fromEnv) {
    return fromEnv;
  }

  return databaseUrl().replace(/\/([^/?]+)(\?.*)?$/, "/$1_shadow$2");
}

function databaseUrlFromPrismaEnv(): string | undefined {
  const envPath = resolve("prisma/.env");
  const contents = readFileSync(envPath, "utf8");
  const line = contents
    .split(/\r?\n/)
    .find((entry) => entry.trim().startsWith("DATABASE_URL="));

  if (!line) {
    return undefined;
  }

  return line
    .slice(line.indexOf("=") + 1)
    .trim()
    .replace(/^['"]|['"]$/g, "");
}
