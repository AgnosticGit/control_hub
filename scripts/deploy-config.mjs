#!/usr/bin/env node
// Renders the files the two production hosts need (docs/operations/deploy.md):
//   <out>/storage/.env, owner.env, seaweedfs-s3.json   -> VPS "storage" (PostgreSQL, ClickHouse, SeaweedFS)
//   <out>/app/.env                                     -> VPS "app" (Kafka, services, web, Caddy)
// Every service password and S3 key is derived from CONTROLHUB_SECRET_SEED (HMAC-SHA256 per
// purpose), so GitHub holds one secret instead of fifteen and every deploy renders the same values.
// Usage: node scripts/deploy-config.mjs <out-dir>   (inputs from the environment, see INPUTS)
import { createHmac } from "node:crypto";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { ARCHIVE_BUCKET, s3Identities } from "./archive-storage.mjs";

export const INPUTS = {
  required: [
    "CONTROLHUB_SECRET_SEED",
    "CONTROLHUB_DOMAIN",
    "STORAGE_PRIVATE_IP",
    "CONTROLHUB_OWNER_EMAIL",
    "CONTROLHUB_OWNER_PASSWORD",
    "IMAGE_PREFIX",
    "CONTROLHUB_VERSION",
  ],
  optional: ["ACME_EMAIL", "CONTROLHUB_LOGS_RETENTION_DAYS", "CONTROLHUB_SPANS_RETENTION_DAYS", "CONTROLHUB_EVENTS_RETENTION_DAYS"],
};

const RETENTION_DEFAULTS = {
  CONTROLHUB_LOGS_RETENTION_DAYS: "14",
  CONTROLHUB_SPANS_RETENTION_DAYS: "14",
  CONTROLHUB_EVENTS_RETENTION_DAYS: "30",
};

/** A stable secret for one purpose; base64url, so it is safe in URLs, SQL and env files. */
export function derive(seed, purpose, length = 32) {
  return createHmac("sha256", seed).update(`controlhub/v1/${purpose}`).digest("base64url").slice(0, length);
}

const hexKey = (seed, purpose) => createHmac("sha256", seed).update(`controlhub/v1/${purpose}`).digest("hex").slice(0, 20);

function validate(env) {
  const missing = INPUTS.required.filter((k) => !env[k]?.trim());
  if (missing.length) throw new Error(`missing: ${missing.join(", ")}`);
  if (env.CONTROLHUB_SECRET_SEED.length < 32) throw new Error("CONTROLHUB_SECRET_SEED must be at least 32 characters");
  if (!/^(?=.{1,253}$)([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$/i.test(env.CONTROLHUB_DOMAIN)) {
    throw new Error("CONTROLHUB_DOMAIN must be a host name such as controlhub.example.com (no scheme, no path)");
  }
  if (!/^(\d{1,3})(\.\d{1,3}){3}$/.test(env.STORAGE_PRIVATE_IP) || env.STORAGE_PRIVATE_IP.split(".").some((o) => Number(o) > 255)) {
    throw new Error("STORAGE_PRIVATE_IP must be an IPv4 address (the storage VPS's private address)");
  }
  if (/['\r\n]/.test(env.CONTROLHUB_OWNER_PASSWORD)) {
    throw new Error("CONTROLHUB_OWNER_PASSWORD must not contain single quotes or line breaks");
  }
  if (!/^[a-z0-9._/-]+$/.test(env.IMAGE_PREFIX)) throw new Error("IMAGE_PREFIX must be a lowercase registry path");
  if (!/^[A-Za-z0-9._-]{1,128}$/.test(env.CONTROLHUB_VERSION)) throw new Error("CONTROLHUB_VERSION is not a valid image tag");
  for (const k of Object.keys(RETENTION_DEFAULTS)) {
    if (env[k] !== undefined && env[k] !== "" && !/^\d{1,4}$/.test(env[k])) throw new Error(`${k} must be a whole number of days`);
  }
}

const envFile = (header, vars) =>
  `# ${header}\n# Rendered by scripts/deploy-config.mjs on every deploy. Never commit.\n${Object.entries(vars)
    .map(([k, v]) => `${k}=${v}`)
    .join("\n")}\n`;

/** The files for both hosts as { "storage/.env": text, ... }. Pure: no I/O. */
export function render(env) {
  validate(env);
  const seed = env.CONTROLHUB_SECRET_SEED;
  const pw = (purpose) => derive(seed, purpose);
  const s3 = {
    CH_S3_ADMIN_ACCESS_KEY: `admin${hexKey(seed, "s3-admin-key")}`,
    CH_S3_ADMIN_SECRET_KEY: pw("s3-admin-secret"),
    CH_S3_ARCHIVER_ACCESS_KEY: `archiver${hexKey(seed, "s3-archiver-key")}`,
    CH_S3_ARCHIVER_SECRET_KEY: pw("s3-archiver-secret"),
    CH_S3_API_ACCESS_KEY: `api${hexKey(seed, "s3-api-key")}`,
    CH_S3_API_SECRET_KEY: pw("s3-api-secret"),
  };
  const retention = Object.fromEntries(
    Object.entries(RETENTION_DEFAULTS).map(([k, d]) => [k, env[k]?.trim() || d]),
  );
  const common = { IMAGE_PREFIX: env.IMAGE_PREFIX, CONTROLHUB_VERSION: env.CONTROLHUB_VERSION, ...retention };
  const db = {
    CH_PG_ADMIN_USER: "controlhub_admin",
    CH_PG_ADMIN_PASSWORD: pw("pg-admin"),
    CH_PG_API_PASSWORD: pw("pg-api"),
    CH_PG_INGEST_PASSWORD: pw("pg-ingest"),
    CH_PG_PROCESSOR_PASSWORD: pw("pg-processor"),
    CH_PG_EXECUTOR_PASSWORD: pw("pg-executor"),
    CH_PG_ARCHIVER_PASSWORD: pw("pg-archiver"),
    CH_CH_ADMIN_USER: "controlhub_admin",
    CH_CH_ADMIN_PASSWORD: pw("ch-admin"),
    CH_CH_API_PASSWORD: pw("ch-api"),
    CH_CH_PROCESSOR_PASSWORD: pw("ch-processor"),
    CH_CH_EXECUTOR_PASSWORD: pw("ch-executor"),
    CH_CH_ARCHIVER_PASSWORD: pw("ch-archiver"),
  };
  const serviceDb = Object.fromEntries(Object.entries(db).filter(([k]) => !k.includes("ADMIN")));
  return {
    "storage/.env": envFile("ControlHub storage host (PostgreSQL, ClickHouse, object storage).", {
      ...common,
      STORAGE_BIND_IP: env.STORAGE_PRIVATE_IP,
      ...db,
      ...s3,
    }),
    // Single quotes: the password is taken literally (no `$` interpolation by Compose).
    "storage/owner.env": `# The owner account created by \`controlhub-api migrate\` (never reset afterwards).\nCONTROLHUB_OWNER_EMAIL=${env.CONTROLHUB_OWNER_EMAIL.trim()}\nCONTROLHUB_OWNER_PASSWORD='${env.CONTROLHUB_OWNER_PASSWORD}'\n`,
    "storage/seaweedfs-s3.json": `${JSON.stringify(s3Identities(s3), null, 2)}\n`,
    "app/.env": envFile("ControlHub app host (Kafka, services, web, Caddy).", {
      ...common,
      CONTROLHUB_DOMAIN: env.CONTROLHUB_DOMAIN,
      ACME_EMAIL: env.ACME_EMAIL?.trim() || env.CONTROLHUB_OWNER_EMAIL.trim(),
      STORAGE_ADDR: env.STORAGE_PRIVATE_IP,
      ARCHIVE_BUCKET,
      ...serviceDb,
      CH_S3_ARCHIVER_ACCESS_KEY: s3.CH_S3_ARCHIVER_ACCESS_KEY,
      CH_S3_ARCHIVER_SECRET_KEY: s3.CH_S3_ARCHIVER_SECRET_KEY,
      CH_S3_API_ACCESS_KEY: s3.CH_S3_API_ACCESS_KEY,
      CH_S3_API_SECRET_KEY: s3.CH_S3_API_SECRET_KEY,
    }),
  };
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  const out = process.argv[2];
  if (!out) {
    console.error("usage: node scripts/deploy-config.mjs <out-dir>");
    process.exit(2);
  }
  let files;
  try {
    files = render(process.env);
  } catch (e) {
    console.error(`[deploy-config] ${e.message}`);
    process.exit(2);
  }
  for (const [rel, text] of Object.entries(files)) {
    const path = join(out, rel);
    mkdirSync(join(path, ".."), { recursive: true });
    writeFileSync(path, text, { mode: 0o600 });
  }
  console.error(`[deploy-config] wrote ${Object.keys(files).length} files to ${out}`);
}
