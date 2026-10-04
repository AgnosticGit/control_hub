// Shared helpers for integration tests against a locally running stack.
import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { readFileSync } from 'node:fs';
import http from 'node:http';
import http2 from 'node:http2';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

export const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const readJson = (name, hint) => {
  try {
    return JSON.parse(readFileSync(join(root, '.karbird', name), 'utf8'));
  } catch {
    throw new Error(`missing .karbird/${name}: ${hint}`);
  }
};
const local = readJson('local-credentials.json', 'run `just bootstrap`');
// Test accounts and the isolation token exist only while `just test-integration` runs.
const fixtures = readJson('test-credentials.json', 'run the suite via `just test-integration`');
export const creds = { ...local, users: fixtures.users, ingest_tokens: { ...local.ingest_tokens, ...fixtures.ingest_tokens } };
export const API = process.env.KARBIRD_API_URL ?? creds.api_url;
export const OTLP = creds.otlp_http_endpoint;
export const ORIGIN = 'http://localhost:3000';
export const WEB = process.env.KARBIRD_WEB_URL ?? ORIGIN;
export const user = (email) => creds.users.find((u) => u.email === email);

/** The developer's own account from the gitignored `.karbird/owner.env`, or null. */
export function ownerAccount() {
  try {
    const env = Object.fromEntries(
      readFileSync(join(root, '.karbird', 'owner.env'), 'utf8')
        .split(/\r?\n/)
        .map((l) => l.trim())
        .filter((l) => l && !l.startsWith('#') && l.includes('='))
        .map((l) => [l.slice(0, l.indexOf('=')), l.slice(l.indexOf('=') + 1)]),
    );
    const email = env.KARBIRD_OWNER_EMAIL?.trim();
    return email && env.KARBIRD_OWNER_PASSWORD ? { email: email.toLowerCase(), password: env.KARBIRD_OWNER_PASSWORD } : null;
  } catch {
    return null;
  }
}

export class Session {
  constructor() {
    this.cookies = new Map();
  }
  cookieHeader() {
    return [...this.cookies].map(([k, v]) => `${k}=${v}`).join('; ');
  }
  async request(path, init = {}) {
    const headers = { ...(init.headers ?? {}) };
    if (this.cookies.size) headers.cookie = this.cookieHeader();
    const res = await fetch(`${API}${path}`, { ...init, headers });
    for (const c of res.headers.getSetCookie()) {
      const [pair] = c.split(';');
      const i = pair.indexOf('=');
      this.cookies.set(pair.slice(0, i), pair.slice(i + 1));
    }
    return res;
  }
  async json(path, init) {
    const res = await this.request(path, init);
    const body = await res.json().catch(() => null);
    return { status: res.status, body };
  }
  /** Cookie-authenticated mutation with Origin and CSRF header. */
  async mutate(path, body, method = 'POST') {
    return this.json(path, {
      method,
      headers: {
        'content-type': 'application/json',
        origin: ORIGIN,
        'x-csrf-token': this.cookies.get('karbird_csrf') ?? '',
      },
      body: JSON.stringify(body ?? {}),
    });
  }
  async login(email) {
    const u = user(email);
    const r = await this.json('/api/v1/session/login', {
      method: 'POST',
      headers: { 'content-type': 'application/json', origin: ORIGIN },
      body: JSON.stringify({ email: u.email, password: u.password }),
    });
    assert.equal(r.status, 200, `login ${email}: ${JSON.stringify(r.body)}`);
    return r.body;
  }
}

export const scoped = (project, env) => `/api/v1/projects/${project}/environments/${env}`;
export const DEMO = scoped('demo', 'local-demo');
export const ISO = scoped('isolation', 'isolation-test');

export async function eventually(fn, { timeoutMs = 30_000, intervalMs = 1000 } = {}) {
  const deadline = Date.now() + timeoutMs;
  let last;
  while (Date.now() < deadline) {
    try {
      return await fn();
    } catch (e) {
      last = e;
      await new Promise((r) => setTimeout(r, intervalMs));
    }
  }
  throw last;
}

export async function bearer(token, path, init = {}) {
  const res = await fetch(`${API}${path}`, {
    ...init,
    headers: { ...(init.headers ?? {}), authorization: `Bearer ${token}` },
  });
  return { status: res.status, body: await res.json().catch(() => null) };
}

export const ADMIN_METRICS = process.env.KARBIRD_INGEST_ADMIN_URL ?? 'http://127.0.0.1:9464';
export const OTLP_GRPC = process.env.KARBIRD_OTLP_GRPC_URL ?? 'http://127.0.0.1:4317';
export const nowNs = () => BigInt(Date.now()) * 1_000_000n;
export const hex = (bytes) => randomBytes(bytes).toString('hex');

/**
 * POSTs a body of `size` bytes, sending only the headers until the server answers, so a
 * server that refuses early by Content-Length (413) is observed instead of a connection reset.
 */
export function postOversized(url, headers, size) {
  const u = new URL(url);
  return new Promise((resolve, reject) => {
    const req = http.request({
      host: u.hostname,
      port: u.port,
      path: u.pathname,
      method: 'POST',
      headers: { ...headers, 'content-length': String(size) },
    });
    req.on('response', (res) => {
      res.resume();
      resolve(res.statusCode);
      req.destroy();
    });
    req.on('error', (e) => (e.code === 'ECONNRESET' || e.code === 'EPIPE' ? resolve(e.code) : reject(e)));
    req.flushHeaders();
    // Give the server a moment to refuse by Content-Length, then stream the body in chunks.
    const chunk = Buffer.alloc(64 * 1024, 0x61);
    let sent = 0;
    const pump = () => {
      while (sent < size && !req.destroyed) {
        const n = Math.min(chunk.length, size - sent);
        sent += n;
        if (!req.write(n === chunk.length ? chunk : chunk.subarray(0, n))) {
          req.once('drain', pump);
          return;
        }
      }
      if (!req.destroyed) req.end();
    };
    setTimeout(pump, 500);
  });
}

// ------------------------------------------------------------ minimal protobuf encoder (OTLP)
// Just enough of the wire format for ExportTraceServiceRequest / ExportLogsServiceRequest,
// so the tests need no protobuf dependency.

function varint(n) {
  let v = BigInt(n);
  const out = [];
  do {
    let b = Number(v & 0x7fn);
    v >>= 7n;
    if (v > 0n) b |= 0x80;
    out.push(b);
  } while (v > 0n);
  return Buffer.from(out);
}
const tag = (field, wire) => varint((field << 3) | wire);
const lenField = (field, buf) => Buffer.concat([tag(field, 2), varint(buf.length), buf]);
const strField = (field, s) => lenField(field, Buffer.from(s, 'utf8'));
const bytesField = (field, b) => lenField(field, b);
const varintField = (field, n) => Buffer.concat([tag(field, 0), varint(n)]);
function fixed64Field(field, n) {
  const b = Buffer.alloc(8);
  b.writeBigUInt64LE(BigInt(n));
  return Buffer.concat([tag(field, 1), b]);
}
const anyString = (s) => strField(1, s);
const keyValues = (field, attrs) =>
  Object.entries(attrs ?? {}).map(([k, v]) => lenField(field, Buffer.concat([strField(1, k), lenField(2, anyString(String(v)))])));
const resource = (attrs) => lenField(1, Buffer.concat(keyValues(1, attrs)));
const scope = (field) => lenField(field, strField(1, 'karbird-integration-tests'));

/**
 * spans: [{ traceId, spanId, parentSpanId?, name, kind?, startNs, endNs, attributes?, status? }]
 * IDs are hex strings (32 / 16 chars). Returns the encoded ExportTraceServiceRequest.
 */
export function encodeTraces(serviceName, spans, resourceAttrs = {}) {
  const encoded = spans.map((s) =>
    lenField(
      2,
      Buffer.concat([
        bytesField(1, Buffer.from(s.traceId, 'hex')),
        bytesField(2, Buffer.from(s.spanId, 'hex')),
        ...(s.parentSpanId ? [bytesField(4, Buffer.from(s.parentSpanId, 'hex'))] : []),
        strField(5, s.name),
        varintField(6, s.kind ?? 2),
        fixed64Field(7, s.startNs),
        fixed64Field(8, s.endNs),
        ...keyValues(9, s.attributes),
        ...(s.status ? [lenField(15, Buffer.concat([strField(2, s.status.message ?? ''), varintField(3, s.status.code ?? 0)]))] : []),
      ]),
    ),
  );
  const resourceSpans = Buffer.concat([resource({ 'service.name': serviceName, ...resourceAttrs }), lenField(2, Buffer.concat([scope(1), ...encoded]))]);
  return lenField(1, resourceSpans);
}

/** logs: [{ timeNs, severity?, severityText?, body, traceId?, spanId?, attributes? }] */
export function encodeLogs(serviceName, logs) {
  const encoded = logs.map((l) =>
    lenField(
      2,
      Buffer.concat([
        fixed64Field(1, l.timeNs),
        varintField(2, l.severity ?? 9),
        strField(3, l.severityText ?? 'INFO'),
        lenField(5, anyString(l.body)),
        ...keyValues(6, l.attributes),
        ...(l.traceId ? [bytesField(9, Buffer.from(l.traceId, 'hex'))] : []),
        ...(l.spanId ? [bytesField(10, Buffer.from(l.spanId, 'hex'))] : []),
        fixed64Field(11, l.timeNs),
      ]),
    ),
  );
  const resourceLogs = Buffer.concat([resource({ 'service.name': serviceName }), lenField(2, Buffer.concat([scope(1), ...encoded]))]);
  return lenField(1, resourceLogs);
}

function doubleField(field, x) {
  const b = Buffer.alloc(8);
  b.writeDoubleLE(x);
  return Buffer.concat([tag(field, 1), b]);
}

/**
 * resources: [{ attributes, metrics: [{ name, unit?, kind: 'gauge' | 'sum' | 'histogram', monotonic?,
 * points: [{ attributes?, value, timeNs, startNs? }] }] }]. Histogram points carry only count and sum
 * (ingest rejects histograms; they exist to test that). Returns an ExportMetricsServiceRequest.
 */
export function encodeMetrics(resources) {
  const point = (p, kind) =>
    Buffer.concat(
      kind === 'histogram'
        ? [
            ...keyValues(9, p.attributes),
            ...(p.startNs ? [fixed64Field(2, p.startNs)] : []),
            fixed64Field(3, p.timeNs),
            fixed64Field(4, 1),
            doubleField(5, p.value),
          ]
        : [...keyValues(7, p.attributes), ...(p.startNs ? [fixed64Field(2, p.startNs)] : []), fixed64Field(3, p.timeNs), doubleField(4, p.value)],
    );
  const metric = (m) => {
    const points = m.points.map((p) => lenField(1, point(p, m.kind)));
    const data =
      m.kind === 'sum'
        ? lenField(7, Buffer.concat([...points, varintField(2, 2), varintField(3, m.monotonic ? 1 : 0)]))
        : m.kind === 'histogram'
          ? lenField(9, Buffer.concat([...points, varintField(2, 2)]))
          : lenField(5, Buffer.concat(points));
    return lenField(2, Buffer.concat([strField(1, m.name), ...(m.unit ? [strField(3, m.unit)] : []), data]));
  };
  return Buffer.concat(
    resources.map((r) => lenField(1, Buffer.concat([resource(r.attributes), lenField(2, Buffer.concat([scope(1), ...r.metrics.map(metric)]))]))),
  );
}

/** Decodes the top-level fields of a protobuf message into { field: [values] } (varints and bytes only). */
export function decodeFields(buf) {
  const out = {};
  let i = 0;
  const readVarint = () => {
    let shift = 0n;
    let v = 0n;
    for (;;) {
      const b = buf[i++];
      v |= BigInt(b & 0x7f) << shift;
      if (!(b & 0x80)) return v;
      shift += 7n;
    }
  };
  while (i < buf.length) {
    const key = Number(readVarint());
    const field = key >> 3;
    const wire = key & 7;
    let value;
    if (wire === 0) value = readVarint();
    else if (wire === 2) {
      const len = Number(readVarint());
      value = buf.subarray(i, i + len);
      i += len;
    } else if (wire === 1) {
      value = buf.readBigUInt64LE(i);
      i += 8;
    } else if (wire === 5) {
      value = buf.readUInt32LE(i);
      i += 4;
    } else throw new Error(`unsupported wire type ${wire}`);
    (out[field] ??= []).push(value);
  }
  return out;
}

/** OTLP/HTTP protobuf export. `signal` is 'traces', 'logs' or 'metrics'. */
export async function otlpHttp(signal, body, { token = creds.ingest_tokens['isolation-test'], headers = {} } = {}) {
  const res = await fetch(`${OTLP}/v1/${signal}`, {
    method: 'POST',
    headers: { authorization: `Bearer ${token}`, 'content-type': 'application/x-protobuf', ...headers },
    body,
  });
  return { status: res.status, contentType: res.headers.get('content-type'), body: Buffer.from(await res.arrayBuffer()) };
}

/** Unary gRPC call over HTTP/2 without a gRPC library. Resolves with grpc-status and the response message. */
export function grpcUnary(service, method, message, metadata = {}) {
  return new Promise((resolve, reject) => {
    const client = http2.connect(OTLP_GRPC);
    client.on('error', reject);
    const frame = Buffer.alloc(5 + message.length);
    frame.writeUInt8(0, 0);
    frame.writeUInt32BE(message.length, 1);
    message.copy(frame, 5);
    const req = client.request({
      ':method': 'POST',
      ':path': `/${service}/${method}`,
      'content-type': 'application/grpc',
      te: 'trailers',
      ...metadata,
    });
    const chunks = [];
    let headers = {};
    req.on('response', (h) => (headers = h));
    req.on('data', (c) => chunks.push(c));
    let trailers = {};
    req.on('trailers', (t) => (trailers = t));
    req.on('end', () => {
      client.close();
      const data = Buffer.concat(chunks);
      const status = Number(trailers['grpc-status'] ?? headers['grpc-status'] ?? -1);
      const messageOut = data.length >= 5 ? data.subarray(5, 5 + data.readUInt32BE(1)) : Buffer.alloc(0);
      resolve({ status, message: trailers['grpc-message'] ?? headers['grpc-message'] ?? '', body: messageOut });
    });
    req.on('error', (e) => {
      client.close();
      reject(e);
    });
    req.end(frame);
  });
}

/** Reads the ingest self-metrics (loopback admin port) into { 'name{labels}': value }. */
export async function ingestMetrics() {
  const text = await fetch(`${ADMIN_METRICS}/metrics`).then((r) => r.text());
  const out = {};
  for (const line of text.split('\n')) {
    const m = /^(\S+)\s+(\d+)$/.exec(line.trim());
    if (m) out[m[1]] = Number(m[2]);
  }
  return out;
}
