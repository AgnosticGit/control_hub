// End-to-end integration test against a locally running stack:
//   just bootstrap && just dev (backend) && just demo
// Verifies SDK -> ingest -> Kafka -> processor -> ClickHouse -> API, plus
// tenant isolation, token scoping and session/CSRF rules.
//
// Run: node --test "tests/integration/*.test.mjs"
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import http from 'node:http';
import { API, OTLP, ORIGIN, WEB, creds, user, Session, DEMO, ISO, eventually, nowNs, hex, encodeTraces, encodeLogs, otlpHttp } from './lib.mjs';

let admin;
before(async () => {
  admin = new Session();
  await admin.login('admin@karbird.local');
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
  // Seed this contract independently of the deliberately low-rate demo generator.
  const session = new Session();
  await session.login('isolation@karbird.local');
  const traceId = hex(16), gatewayId = hex(8), ordersId = hex(8);
  const at = nowNs() - 60_000_000_000n;
  assert.equal((await otlpHttp('traces', encodeTraces('e2e-gateway', [{
    traceId, spanId: gatewayId, name: 'GET /orders', kind: 2, startNs: at, endNs: at + 20_000_000n,
  }]))).status, 200);
  assert.equal((await otlpHttp('traces', encodeTraces('e2e-orders', [{
    traceId, spanId: ordersId, parentSpanId: gatewayId, name: 'SELECT orders', kind: 2, startNs: at + 2_000_000n, endNs: at + 15_000_000n,
    attributes: { 'db.system': 'postgresql' },
  }]))).status, 200);
  assert.equal((await otlpHttp('logs', encodeLogs('e2e-orders', [{
    timeNs: at + 10_000_000n, body: 'correlated order lookup', traceId, spanId: ordersId,
  }]))).status, 200);
  const multi = await eventually(async () => {
    const list = await session.json(`${ISO}/traces?limit=50`);
    assert.equal(list.status, 200);
    const found = list.body.traces.find((t) => t.trace_id === traceId && t.services.includes('e2e-gateway') && t.services.includes('e2e-orders'));
    assert.ok(found, 'expected the seeded trace to span e2e-gateway and e2e-orders');
    return found;
  });
  const detail = await session.json(`${ISO}/traces/${multi.trace_id}`);
  assert.equal(detail.status, 200);
  const services = new Set(detail.body.spans.map((s) => s.service_name));
  assert.ok(services.has('e2e-gateway') && services.has('e2e-orders'));
  for (const s of detail.body.spans) {
    for (const k of Object.keys(s.attributes)) {
      assert.ok(!/authorization|cookie|db\.statement|db\.query\.text/i.test(k), `sensitive attribute leaked: ${k}`);
    }
  }
});

test('timeline returns engine-compatible items with nanosecond strings', async () => {
  // Demo events are infrequent and can fall outside its rolling window. Seed all three
  // signals in our own exact range so this contract test never depends on demo uptime.
  const session = new Session();
  await session.login('isolation@karbird.local');
  const at = nowNs() - 2n * 86_400_000_000_000n;
  const service = `timeline-contract-${hex(4)}`;
  const traceId = hex(16), spanId = hex(8), eventId = randomUUID();
  assert.equal((await otlpHttp('traces', encodeTraces(service, [{ traceId, spanId, name: service, kind: 2, startNs: at, endNs: at + 1_000_000n }]))).status, 200);
  assert.equal((await otlpHttp('logs', encodeLogs(service, [{ timeNs: at, body: service, traceId, spanId }]))).status, 200);
  const event = await fetch(`${OTLP}/api/v1/events/batch`, {
    method: 'POST',
    headers: { authorization: `Bearer ${creds.ingest_tokens['isolation-test']}`, 'content-type': 'application/json' },
    body: JSON.stringify({ events: [{ event_id: eventId, event_type: 'deployment.finished', service: { name: service }, event_time: `${at}` }] }),
  });
  assert.equal(event.status, 200);
  assert.equal((await event.json()).accepted, 1);
  await eventually(async () => {
    const { status, body } = await session.json(`${ISO}/timeline?from=${at - 1n}&to=${at + 2_000_000n}&limit=2000`);
    assert.equal(status, 200);
    const types = new Set(body.items.map((i) => i.type));
    assert.ok(types.has('span') && types.has('log') && types.has('event'), `types: ${[...types]}`);
    const span = body.items.find((i) => i.type === 'span' && i.trace_id === traceId);
    assert.ok(span, 'the seeded trace must be returned');
    assert.match(span.start_ns, /^\d+$/);
    assert.ok(typeof body.completeness.processing_caught_up === 'boolean');
  });
});

test('unauthenticated requests are rejected', async () => {
  const r = await fetch(`${API}${DEMO}/services`);
  assert.equal(r.status, 401);
});

test('login rejects foreign Origin', async () => {
  const u = user('viewer@karbird.local');
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
    body: JSON.stringify({ email: 'viewer@karbird.local', password: 'wrong' }),
  });
  const b = await fetch(`${API}/api/v1/session/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ email: `nobody-${randomUUID()}@karbird.local`, password: 'wrong' }),
  });
  assert.equal(a.status, 401);
  assert.equal(b.status, 401);
  assert.deepEqual(await a.json(), await b.json());
});

test('logout requires CSRF token', async () => {
  const s = new Session();
  await s.login('viewer@karbird.local');
  const noCsrf = await s.request('/api/v1/session/logout', { method: 'POST', headers: { origin: ORIGIN } });
  assert.equal(noCsrf.status, 403);
  const ok = await s.request('/api/v1/session/logout', {
    method: 'POST',
    headers: { origin: ORIGIN, 'x-csrf-token': s.cookies.get('karbird_csrf') },
  });
  assert.equal(ok.status, 204);
  const after = await s.json('/api/v1/session');
  assert.equal(after.status, 401);
});

test('isolation user cannot see demo project and demo user cannot see isolation project', async () => {
  const iso = new Session();
  const sess = await iso.login('isolation@karbird.local');
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
  await iso.login('isolation@karbird.local');
  await eventually(async () => {
    const r = await iso.json(`${ISO}/events`);
    assert.equal(r.status, 200);
    assert.ok(r.body.events.some((e) => e.service_name === marker), 'event visible in isolation scope');
  });
  const demoEvents = await admin.json(`${DEMO}/events?limit=1000`);
  assert.ok(!demoEvents.body.events.some((e) => e.service_name === marker), 'event leaked into demo scope');
});

/** Raw WebSocket handshake; resolves with the HTTP status (101 on success). */
function handshake(base, path, headers) {
  const u = new URL(path, base);
  return new Promise((resolve, reject) => {
    const req = http.request({
      host: u.hostname,
      port: u.port,
      path: u.pathname + u.search,
      headers: {
        connection: 'Upgrade',
        upgrade: 'websocket',
        'sec-websocket-version': '13',
        'sec-websocket-key': Buffer.from(randomUUID().slice(0, 16)).toString('base64'),
        ...headers,
      },
    });
    req.on('upgrade', (res, socket) => {
      socket.destroy();
      resolve(res.statusCode);
    });
    req.on('response', (res) => {
      res.resume();
      resolve(res.statusCode);
    });
    req.on('error', reject);
    req.end();
  });
}

/** Opens the live channel and collects messages until `done(messages)` returns true. */
function liveMessages(base, session, done, timeoutMs = 20_000, { infrastructure = false } = {}) {
  const url = new URL(`${DEMO}/live`, base);
  if (infrastructure) url.searchParams.set('infrastructure', 'true');
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  const ws = new WebSocket(url, { headers: { origin: ORIGIN, cookie: session.cookieHeader() } });
  const messages = [];
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      ws.close();
      reject(new Error(`timeout; got ${messages.map((m) => m.type)}`));
    }, timeoutMs);
    ws.onmessage = (ev) => {
      messages.push(JSON.parse(ev.data));
      if (done(messages)) {
        clearTimeout(timer);
        ws.close();
        resolve(messages);
      }
    };
    ws.onerror = () => {
      clearTimeout(timer);
      reject(new Error('websocket error'));
    };
  });
}

test('live WebSocket requires session, allowed Origin and project access', async () => {
  const path = `${DEMO}/live`;
  assert.equal(await handshake(API, path, { origin: ORIGIN }), 401, 'no session');
  assert.equal(await handshake(API, path, { cookie: admin.cookieHeader() }), 403, 'no Origin');
  assert.equal(await handshake(API, path, { cookie: admin.cookieHeader(), origin: 'https://evil.example' }), 403);
  const iso = new Session();
  await iso.login('isolation@karbird.local');
  assert.equal(await handshake(API, path, { cookie: iso.cookieHeader(), origin: ORIGIN }), 404, 'foreign project');
  assert.equal(await handshake(API, path, { cookie: admin.cookieHeader(), origin: ORIGIN }), 101);
});

test('API metrics expose aggregate live pipeline health without tenant labels', async () => {
  const response = await fetch(`${API}/metrics`);
  assert.equal(response.status, 200);
  assert.match(response.headers.get('content-type') ?? '', /text\/plain.*version=0\.0\.4/);
  const body = await response.text();
  for (const metric of [
    'karbird_live_kafka_records_total',
    'karbird_live_kafka_decode_errors_total',
    'karbird_live_consumer_lag_messages',
    'karbird_live_hub_subscriber_overflows_total',
  ]) assert.match(body, new RegExp(`^${metric} \\d+$`, 'm'));
  assert.doesNotMatch(body, /tenant_id|project_id|environment_id/);
  const publicResponse = await fetch(`${WEB}/metrics`);
  assert.equal(publicResponse.status, 404, 'the public web entry point must not proxy internal API metrics');
});

for (const [name, base] of [
  ['API', API],
  ['web proxy', WEB],
]) {
  test(`live WebSocket pushes individually framed records for the demo environment (${name})`, async (t) => {
    if (base === WEB && !(await fetch(WEB).then(() => true, () => false))) {
      t.skip(`web server not running at ${WEB}`);
      return;
    }
    const msgs = await liveMessages(base, admin, (m) => m.some((x) => ['span', 'log', 'event'].includes(x.type)));
    assert.equal(msgs[0].type, 'hello');
    assert.match(msgs[0].resume, /^\d+$/);
    const records = msgs.filter((m) => ['span', 'log', 'event'].includes(m.type));
    assert.ok(records.every((m) => BigInt(m.resume) >= BigInt(msgs[0].resume)), 'resume cursor moves forward');
    assert.ok(records.every((m) => m.item && m.record), 'each frame has one engine item and one full record');
    assert.ok(records.every((m) => !Array.isArray(m.items) && !Array.isArray(m.signals)), 'records are not grouped into batches');
    const types = new Set(records.map((m) => m.type));
    assert.ok(types.has('span') || types.has('log'), `types: ${[...types]}`);
    assert.ok(records.every((m) => m.record?.service_id), 'live records are typed and scoped');
  });
}

test('live WebSocket sends the initial infrastructure snapshot when requested', async () => {
  const msgs = await liveMessages(API, admin, (m) => m.some((x) => x.type === 'infrastructure'), 25_000, { infrastructure: true });
  const snapshot = msgs.find((message) => message.type === 'infrastructure');
  assert.ok(snapshot.snapshot && typeof snapshot.snapshot === 'object');
  assert.match(snapshot.resume, /^\d+$/);
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
