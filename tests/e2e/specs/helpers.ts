import { randomBytes } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, type Page } from "@playwright/test";

interface Creds {
  users: Array<{ email: string; password: string; project_id: string; role: string }>;
  ingest_tokens: Record<string, string>;
  otlp_http_endpoint: string;
}

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const creds = JSON.parse(readFileSync(join(root, ".controlhub", "local-credentials.json"), "utf8")) as Creds;

export function user(email: string) {
  const u = creds.users.find((x) => x.email === email);
  if (!u) throw new Error(`no local credentials for ${email}; run just bootstrap`);
  return u;
}

export async function login(page: Page, email: string, next = "/app") {
  const u = user(email);
  await page.goto(`/app/login?next=${encodeURIComponent(next)}`);
  await page.getByLabel("Email").fill(u.email);
  await page.getByLabel("Password").fill(u.password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page).not.toHaveURL(/\/app\/login/);
}

/** Uncaught page errors and console errors, for "renders without errors" checks. */
export function collectErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(`console: ${m.text()}`);
  });
  return errors;
}

// ------------------------------------------------ OTLP/HTTP protobuf log export (no deps)

function varint(n: bigint | number): Buffer {
  let v = BigInt(n);
  const out: number[] = [];
  do {
    let b = Number(v & 0x7fn);
    v >>= 7n;
    if (v > 0n) b |= 0x80;
    out.push(b);
  } while (v > 0n);
  return Buffer.from(out);
}
const tag = (field: number, wire: number) => varint((field << 3) | wire);
const len = (field: number, buf: Buffer) => Buffer.concat([tag(field, 2), varint(buf.length), buf]);
const str = (field: number, s: string) => len(field, Buffer.from(s, "utf8"));
function fixed64(field: number, n: bigint): Buffer {
  const b = Buffer.alloc(8);
  b.writeBigUInt64LE(n);
  return Buffer.concat([tag(field, 1), b]);
}

export const randomHex = (bytes: number) => randomBytes(bytes).toString("hex");

/**
 * Sends one log record through the real ingest endpoint with the environment's ingestion
 * token (`isolation-test` by default, so the demo map stays clean).
 */
export async function ingestLog(opts: {
  service: string;
  body: string;
  severity?: number;
  traceId?: string;
  env?: string;
  /** Explicit token and OTLP base URL, e.g. a token created in the UI and the site's /otlp proxy. */
  token?: string;
  endpoint?: string;
}) {
  const now = BigInt(Date.now()) * 1_000_000n;
  const record = Buffer.concat([
    fixed64(1, now),
    tag(2, 0),
    varint(opts.severity ?? 13),
    str(3, (opts.severity ?? 13) >= 17 ? "ERROR" : "WARN"),
    len(5, str(1, opts.body)),
    ...(opts.traceId ? [len(9, Buffer.from(opts.traceId, "hex"))] : []),
    fixed64(11, now),
  ]);
  const resource = len(1, len(1, Buffer.concat([str(1, "service.name"), len(2, str(1, opts.service))])));
  const body = len(1, Buffer.concat([resource, len(2, len(2, record))]));
  const token = opts.token ?? creds.ingest_tokens[opts.env ?? "isolation-test"];
  const res = await fetch(`${opts.endpoint ?? creds.otlp_http_endpoint}/v1/logs`, {
    method: "POST",
    headers: { authorization: `Bearer ${token}`, "content-type": "application/x-protobuf" },
    body,
  });
  if (res.status !== 200) throw new Error(`log ingest failed: ${res.status}`);
}
