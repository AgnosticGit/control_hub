// End-to-end integration test against a locally running stack:
//   just bootstrap && just dev (backend) && just demo
// Verifies SDK -> ingest -> Kafka -> processor -> ClickHouse -> API, plus
// tenant isolation, token scoping and session/CSRF rules.
//
// Run: node --test tests/integration/
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const creds = JSON.parse(readFileSync(join(root, '.controlhub', 'local-credentials.json'), 'utf8'));
const API = process.env.CONTROLHUB_API_URL ?? creds.api_url;
const OTLP = creds.otlp_http_endpoint;
const ORIGIN = 'http://localhost:3000';
const user = (email) => creds.users.find((u) => u.email === email);

class Session {
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

const scoped = (project, env) => `/api/v1/projects/${project}/environments/${env}`;
const DEMO = scoped('demo', 'local-demo');
const ISO = scoped('isolation', 'isolation-test');

async function eventually(fn, { timeoutMs = 30_000, intervalMs = 1000 } = {}) {
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

let admin;
before(async () => {
  admin = new Session();
  await admin.login('admin@controlhub.local');
});

test('demo services, logs and events are queryable through the API', async () => {
  await eventually(async () => {
    const r = await admin.json(`${DEMO}/services`);
    assert.equal(r.status, 200);
    assert.equal(r.body.is_demo, true);
    const names = r.body.services.map((s) => s.name).sort();
    assert.ok(names.includes('gateway') && names.includes('orders'), `services: ${names}`);
    const orders = r.body.services.find((s) => s.name === 'orders');
    assert.ok(orders.requests > 0);
    assert.ok(orders.logs > 0, 'orders must have logs');
  });
});

test('topology has observed gateway->orders call and inferred orders->postgres dependency', async () => {
  await eventually(async () => {
    const { status, body } = await admin.json(`${DEMO}/topology`);
    assert.equal(status, 200);
    const byId = new Map(body.nodes.map((n) => [n.id, n]));
    const edge = (from, to) =>
      body.edges.find((e) => byId.get(e.source)?.name === from && byId.get(e.target)?.name?.includes(to));
    const call = edge('gateway', 'orders');
    assert.ok(call, `edges: ${JSON.stringify(body.edges.map((e) => [byId.get(e.source)?.name, byId.get(e.target)?.name]))}`);
    assert.equal(call.provenance, 'observed');
    const db = body.edges.find((e) => byId.get(e.source)?.name === 'orders' && byId.get(e.target)?.kind !== 'service');
    assert.ok(db, 'orders -> database edge');
    assert.equal(db.provenance, 'inferred');
    assert.ok(db.reasons.length > 0, 'inferred edge must carry reasons');
  });
});

test('trace detail links spans across services with correlated logs', async () => {
  const list = await admin.json(`${DEMO}/traces?limit=50`);
  assert.equal(list.status, 200);
  const multi = list.body.traces.find((t) => t.services.length >= 2);
  assert.ok(multi, 'expected a trace spanning gateway and orders');
  const detail = await admin.json(`${DEMO}/traces/${multi.trace_id}`);
  assert.equal(detail.status, 200);
  const services = new Set(detail.body.spans.map((s) => s.service_name));
  assert.ok(services.has('gateway') && services.has('orders'));
  for (const s of detail.body.spans) {
    for (const k of Object.keys(s.attributes)) {
      assert.ok(!/authorization|cookie|db\.statement|db\.query\.text/i.test(k), `sensitive attribute leaked: ${k}`);
    }
  }
});

test('timeline returns engine-compatible items with nanosecond strings', async () => {
  const { status, body } = await admin.json(`${DEMO}/timeline?limit=2000`);
  assert.equal(status, 200);
  assert.ok(body.items.length > 0);
  const types = new Set(body.items.map((i) => i.type));
  assert.ok(types.has('span') && types.has('log') && types.has('event'), `types: ${[...types]}`);
  const span = body.items.find((i) => i.type === 'span');
  assert.match(span.start_ns, /^\d+$/);
  assert.ok(typeof body.completeness.processing_caught_up === 'boolean');
});

test('unauthenticated requests are rejected', async () => {
  const r = await fetch(`${API}${DEMO}/services`);
  assert.equal(r.status, 401);
});

test('login rejects foreign Origin', async () => {
  const u = user('viewer@controlhub.local');
  const r = await fetch(`${API}/api/v1/session/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: 'https://evil.example' },
    body: JSON.stringify({ email: u.email, password: u.password }),
  });
  assert.equal(r.status, 403);
});

test('wrong password is rejected without revealing account existence', async () => {
  const a = await fetch(`${API}/api/v1/session/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ email: 'viewer@controlhub.local', password: 'wrong' }),
  });
  const b = await fetch(`${API}/api/v1/session/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ email: `nobody-${randomUUID()}@controlhub.local`, password: 'wrong' }),
  });
  assert.equal(a.status, 401);
  assert.equal(b.status, 401);
  assert.deepEqual(await a.json(), await b.json());
});

test('logout requires CSRF token', async () => {
  const s = new Session();
  await s.login('viewer@controlhub.local');
  const noCsrf = await s.request('/api/v1/session/logout', { method: 'POST', headers: { origin: ORIGIN } });
  assert.equal(noCsrf.status, 403);
  const ok = await s.request('/api/v1/session/logout', {
    method: 'POST',
    headers: { origin: ORIGIN, 'x-csrf-token': s.cookies.get('controlhub_csrf') },
  });
  assert.equal(ok.status, 204);
  const after = await s.json('/api/v1/session');
  assert.equal(after.status, 401);
});

test('isolation user cannot see demo project and demo user cannot see isolation project', async () => {
  const iso = new Session();
  const sess = await iso.login('isolation@controlhub.local');
  assert.deepEqual(sess.projects.map((p) => p.id), ['isolation']);
  for (const path of ['services', 'topology', 'traces', 'logs', 'events', 'timeline']) {
    const r = await iso.json(`${DEMO}/${path}`);
    assert.equal(r.status, 404, `isolation user got ${r.status} on demo ${path}`);
  }
  const r = await admin.json(`${ISO}/services`);
  assert.equal(r.status, 404);
});

test('ingestion scope comes from the token, not from payload', async () => {
  const marker = `spoof-${randomUUID().slice(0, 8)}`;
  const res = await fetch(`${OTLP}/api/v1/events/batch`, {
    method: 'POST',
    headers: {
      authorization: `Bearer ${creds.ingest_tokens['isolation-test']}`,
      'content-type': 'application/json',
    },
    body: JSON.stringify({
      events: [
        {
          event_id: randomUUID(),
          event_type: 'deployment.started',
          service: { name: marker },
          event_time: `${BigInt(Date.now()) * 1_000_000n}`,
          attributes: { tenant_id: 'local', project_id: 'demo', environment_id: 'local-demo' },
        },
      ],
    }),
  });
  const ack = await res.json();
  assert.equal(res.status, 200, JSON.stringify(ack));
  assert.equal(ack.accepted, 1);
  assert.equal(ack.durability, 'broker_acknowledged');

  const iso = new Session();
  await iso.login('isolation@controlhub.local');
  await eventually(async () => {
    const r = await iso.json(`${ISO}/events`);
    assert.equal(r.status, 200);
    assert.ok(r.body.events.some((e) => e.service_name === marker), 'event visible in isolation scope');
  });
  const demoEvents = await admin.json(`${DEMO}/events?limit=1000`);
  assert.ok(!demoEvents.body.events.some((e) => e.service_name === marker), 'event leaked into demo scope');
});

test('ingestion rejects missing and invalid tokens', async () => {
  const none = await fetch(`${OTLP}/api/v1/events/batch`, { method: 'POST', body: '{"events":[]}' });
  assert.equal(none.status, 401);
  const bad = await fetch(`${OTLP}/v1/traces`, {
    method: 'POST',
    headers: { authorization: 'Bearer chi_invalid', 'content-type': 'application/x-protobuf' },
    body: new Uint8Array(),
  });
  assert.equal(bad.status, 401);
  const json = await fetch(`${OTLP}/v1/traces`, {
    method: 'POST',
    headers: { authorization: `Bearer ${creds.ingest_tokens['demo-local']}`, 'content-type': 'application/json' },
    body: '{}',
  });
  assert.equal(json.status, 415, 'OTLP/JSON is explicitly unsupported');
});
