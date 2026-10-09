import { randomBytes } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, type Locator, type Page } from "@playwright/test";

interface Creds {
  users: Array<{ email: string; password: string; project_id: string; role: string }>;
  ingest_tokens: Record<string, string>;
  otlp_http_endpoint: string;
}

export const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
function readJson<T>(name: string, hint: string): T {
  try {
    return JSON.parse(readFileSync(join(root, ".karbird", name), "utf8")) as T;
  } catch {
    throw new Error(`missing .karbird/${name}: ${hint}`);
  }
}
const local = readJson<Omit<Creds, "users">>("local-credentials.json", "run `just bootstrap`");
// Test accounts and the isolation token exist only while `just test-e2e` runs.
const fixtures = readJson<Pick<Creds, "users" | "ingest_tokens">>("test-credentials.json", "run the suite via `just test-e2e`");
const creds: Creds = { ...local, users: fixtures.users, ingest_tokens: { ...local.ingest_tokens, ...fixtures.ingest_tokens } };

/** The developer's own account from the gitignored `.karbird/owner.env`, or null. */
export function ownerAccount(): { email: string; password: string } | null {
  let text: string;
  try {
    text = readFileSync(join(root, ".karbird", "owner.env"), "utf8");
  } catch {
    return null;
  }
  const env = new Map<string, string>();
  for (const line of text.split(/\r?\n/)) {
    const t = line.trim();
    const i = t.indexOf("=");
    if (t && !t.startsWith("#") && i > 0) env.set(t.slice(0, i), t.slice(i + 1));
  }
  const email = env.get("KARBIRD_OWNER_EMAIL")?.trim().toLowerCase();
  const password = env.get("KARBIRD_OWNER_PASSWORD");
  return email && password ? { email, password } : null;
}

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

/** Opens the map's layers popover before returning a filter checkbox. */
export async function mapLayer(page: Page, name: string): Promise<Locator> {
  const checkbox = page.getByRole("checkbox", { name });
  if (!(await checkbox.isVisible())) await page.getByRole("button", { name: "Map layers" }).click();
  return checkbox;
}

/** Selects a replay speed through the slider's real keyboard interaction. */
export async function setReplaySpeed(page: Page, value: string) {
  const speed = Number(value);
  if (!Number.isFinite(speed) || speed < 0.001 || speed > 10) throw new Error(`unsupported replay speed ${value}`);
  const position = Math.round(Math.log10(speed / 0.001) * 250);
  const selectedSpeed = Number((0.001 * 10 ** (position / 250)).toPrecision(4)).toString();
  const slider = page.getByRole("slider", { name: "Replay speed" });
  await slider.focus();
  const fromEnd = 1000 - position;
  await page.keyboard.press(fromEnd < position ? "End" : "Home");
  const direction = fromEnd < position ? "ArrowLeft" : "ArrowRight";
  for (let step = 0; step < Math.min(fromEnd, position); step++) await page.keyboard.press(direction);
  await expect(slider).toHaveAttribute("aria-valuetext", `${selectedSpeed}×`);
}

/** The page's own alert; skips Next.js's empty route announcer, which also has role=alert. */
export function pageAlert(page: Page) {
  return page.getByRole("alert").filter({ hasText: /\S/ });
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
  /** Record time (unix ns); now by default. */
  timeNs?: bigint;
}) {
  const now = opts.timeNs ?? BigInt(Date.now()) * 1_000_000n;
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

export interface SpanInput {
  traceId: string;
  spanId: string;
  parentSpanId?: string;
  name: string;
  /** OTLP SpanKind: 2 server, 3 client. */
  kind: number;
  startNs: bigint;
  endNs: bigint;
  attributes?: Record<string, string>;
}

const keyValue = (field: number, k: string, v: string) => len(field, Buffer.concat([str(1, k), len(2, str(1, v))]));

export interface MetricPoint {
  attributes?: Record<string, string>;
  value: number;
  timeNs: bigint;
  /** Start of a cumulative sum. */
  startNs?: bigint;
}

export interface MetricInput {
  name: string;
  unit?: string;
  kind: "gauge" | "sum";
  monotonic?: boolean;
  points: MetricPoint[];
}

function double(field: number, x: number): Buffer {
  const b = Buffer.alloc(8);
  b.writeDoubleLE(x);
  return Buffer.concat([tag(field, 1), b]);
}

/** Sends agent-style metrics (gauges and sums, one resource each) through the real ingest endpoint. */
export async function ingestMetrics(resources: Array<{ attributes: Record<string, string>; metrics: MetricInput[] }>, env = "isolation-test") {
  const point = (p: MetricPoint) =>
    Buffer.concat([
      ...Object.entries(p.attributes ?? {}).map(([k, v]) => keyValue(7, k, v)),
      ...(p.startNs ? [fixed64(2, p.startNs)] : []),
      fixed64(3, p.timeNs),
      double(4, p.value),
    ]);
  const metric = (m: MetricInput) => {
    const points = m.points.map((p) => len(1, point(p)));
    const data =
      m.kind === "sum"
        ? len(7, Buffer.concat([...points, tag(2, 0), varint(2), tag(3, 0), varint(m.monotonic ? 1 : 0)]))
        : len(5, Buffer.concat(points));
    return len(2, Buffer.concat([str(1, m.name), ...(m.unit ? [str(3, m.unit)] : []), data]));
  };
  const body = Buffer.concat(
    resources.map((r) =>
      len(
        1,
        Buffer.concat([
          len(1, Buffer.concat(Object.entries(r.attributes).map(([k, v]) => keyValue(1, k, v)))),
          len(2, Buffer.concat(r.metrics.map(metric))),
        ]),
      ),
    ),
  );
  const res = await fetch(`${creds.otlp_http_endpoint}/v1/metrics`, {
    method: "POST",
    headers: { authorization: `Bearer ${creds.ingest_tokens[env]}`, "content-type": "application/x-protobuf" },
    body,
  });
  if (res.status !== 200) throw new Error(`metric ingest failed: ${res.status}`);
  if ((await res.arrayBuffer()).byteLength !== 0) throw new Error("metric ingest refused some points (partial_success)");
}

/** Sends spans of one service through the real ingest endpoint (`isolation-test` by default). */
export async function ingestSpans(service: string, spans: SpanInput[], env = "isolation-test", resourceAttributes: Record<string, string> = {}) {
  const encoded = spans.map((s) =>
    len(
      2,
      Buffer.concat([
        len(1, Buffer.from(s.traceId, "hex")),
        len(2, Buffer.from(s.spanId, "hex")),
        ...(s.parentSpanId ? [len(4, Buffer.from(s.parentSpanId, "hex"))] : []),
        str(5, s.name),
        tag(6, 0),
        varint(s.kind),
        fixed64(7, s.startNs),
        fixed64(8, s.endNs),
        ...Object.entries(s.attributes ?? {}).map(([k, v]) => keyValue(9, k, v)),
      ]),
    ),
  );
  const resource = len(
    1,
    Buffer.concat([keyValue(1, "service.name", service), ...Object.entries(resourceAttributes).map(([k, v]) => keyValue(1, k, v))]),
  );
  const body = len(1, Buffer.concat([resource, len(2, Buffer.concat(encoded))]));
  const res = await fetch(`${creds.otlp_http_endpoint}/v1/traces`, {
    method: "POST",
    headers: { authorization: `Bearer ${creds.ingest_tokens[env]}`, "content-type": "application/x-protobuf" },
    body,
  });
  if (res.status !== 200) throw new Error(`trace ingest failed: ${res.status}`);
}
