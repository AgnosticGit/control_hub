// Data flowing over a map edge: GET …/edges/traffic sums the HTTP body sizes spans declare
// (`http.request.body.size` / `http.response.body.size`, from Content-Length) per bucket and
// reports the average and peak throughput. A service call takes the callee's sizes and falls
// back to the caller's; calls to a database declare none, so their rate is unknown, not zero.
// Writes only into the isolation tenant with unique names.
// Run: node --test "tests/integration/*.test.mjs"
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { API, DEMO, ISO, Session, encodeTraces, eventually, hex, nowNs, otlpHttp } from './lib.mjs';

const SEC = 1_000_000_000n;
const MS = 1_000_000n;
const x = hex(4);
const GW = `it-et-gw-${x}`;
const ORDERS = `it-et-orders-${x}`;
const DB = `it_et_${x}`;
const T0 = nowNs() - 120n * SEC;
const FROM = T0 - 10n * SEC;
const TO = T0 + 50n * SEC;

let iso;
let admin;
before(async () => {
  iso = new Session();
  await iso.login('isolation@karbird.local');
  admin = new Session();
  await admin.login('admin@karbird.local');
});

const sizes = (req, resp) => ({ 'http.request.body.size': String(req), 'http.response.body.size': String(resp) });

/**
 * Four requests, one every 10 s from T0: browser → gw (root server span, 200 B in / 800 B out)
 * → gw client span → orders server span (1000 B / 4000 B; the last one reports nothing, its
 * caller 500 B / 2000 B), plus two gw queries to PostgreSQL without sizes.
 */
function spans() {
  const gw = [];
  const orders = [];
  for (let i = 0n; i < 4n; i++) {
    const start = T0 + i * 10n * SEC;
    const [traceId, root, client] = [hex(16), hex(8), hex(8)];
    gw.push({ traceId, spanId: root, name: 'GET /checkout', kind: 2, startNs: start, endNs: start + 40n * MS, attributes: sizes(200, 800) });
    const last = i === 3n;
    gw.push({
      traceId,
      spanId: client,
      parentSpanId: root,
      name: 'POST',
      kind: 3,
      startNs: start + MS,
      endNs: start + 30n * MS,
      attributes: last ? sizes(500, 2000) : {},
    });
    orders.push({
      traceId,
      spanId: hex(8),
      parentSpanId: client,
      name: 'POST /orders',
      kind: 2,
      startNs: start + 2n * MS,
      endNs: start + 28n * MS,
      attributes: last ? {} : sizes(1000, 4000),
    });
    if (i < 2n) {
      gw.push({
        traceId,
        spanId: hex(8),
        parentSpanId: root,
        name: 'SELECT',
        kind: 3,
        startNs: start + 31n * MS,
        endNs: start + 35n * MS,
        attributes: { 'db.system': 'postgresql', 'db.name': DB, 'server.address': 'localhost', 'server.port': '5432' },
      });
    }
  }
  return { gw, orders };
}

const traffic = (scope, session, q) =>
  session.json(`${scope}/edges/traffic?${new URLSearchParams({ from: String(FROM), to: String(TO), buckets: '6', ...q })}`);

let ids;
async function nodeIds() {
  if (ids) return ids;
  const { gw, orders } = spans();
  assert.equal((await otlpHttp('traces', encodeTraces(GW, gw))).status, 200);
  assert.equal((await otlpHttp('traces', encodeTraces(ORDERS, orders))).status, 200);
  ids = await eventually(
    async () => {
      const r = await iso.json(`${ISO}/topology?from=${FROM}&to=${TO}`);
      assert.equal(r.status, 200, JSON.stringify(r.body));
      const id = (pred) => r.body.nodes.find(pred)?.id;
      const found = {
        gw: id((n) => n.name === GW),
        orders: id((n) => n.name === ORDERS),
        db: id((n) => n.kind === 'database' && n.name.includes(DB)),
        clients: id((n) => n.kind === 'external_client'),
      };
      for (const [k, v] of Object.entries(found)) assert.ok(v, `${k} node in ${JSON.stringify(r.body.nodes.map((n) => n.name))}`);
      const call = r.body.edges.find((e) => e.source === found.gw && e.target === found.orders);
      assert.equal(call?.calls, 4, 'every call processed');
      return found;
    },
    { timeoutMs: 60_000, intervalMs: 2000 },
  );
  return ids;
}

test('a service call carries the callee sizes, falling back to the caller, with average and peak', async () => {
  const n = await nodeIds();
  const r = await traffic(ISO, iso, { source: n.gw, target: n.orders, relation: 'calls' });
  assert.equal(r.status, 200, JSON.stringify(r.body));
  const t = r.body;
  assert.equal(t.bucket_ns, String(10n * SEC));
  assert.deepEqual(t.calls, [0, 1, 1, 1, 1, 0]);
  assert.deepEqual([t.total_calls, t.total_sized_calls, t.size_coverage], [4, 4, 1]);
  assert.deepEqual([t.total_request_bytes, t.total_response_bytes], [3 * 1000 + 500, 3 * 4000 + 2000]);
  assert.deepEqual(t.response_bytes, [0, 4000, 4000, 4000, 2000, 0]);
  assert.ok(Math.abs(t.avg_response_bps - 14000 / 60) < 1e-6, String(t.avg_response_bps));
  assert.equal(t.peak_response_bps, 400);
  assert.equal(t.peak_response_at_ns, String(FROM + 10n * SEC), 'first of the equal peaks');
});

test('requests from outside are measured on the root server spans', async () => {
  const n = await nodeIds();
  const r = await traffic(ISO, iso, { source: n.clients, target: n.gw, relation: 'calls' });
  assert.equal(r.status, 200, JSON.stringify(r.body));
  assert.deepEqual([r.body.total_calls, r.body.total_request_bytes, r.body.total_response_bytes], [4, 800, 3200]);
  assert.equal(r.body.peak_request_bps, 20);
});

test('a database link without declared sizes reports calls but an unknown rate', async () => {
  const n = await nodeIds();
  const r = await traffic(ISO, iso, { source: n.gw, target: n.db, relation: 'depends_on' });
  assert.equal(r.status, 200, JSON.stringify(r.body));
  assert.deepEqual([r.body.total_calls, r.body.total_sized_calls, r.body.size_coverage], [2, 0, 0]);
  for (const k of ['avg_request_bps', 'avg_response_bps', 'peak_request_bps', 'peak_response_bps']) {
    assert.equal(r.body[k] ?? null, null, `${k} is unknown, not zero`);
  }
});

test('edge traffic validates input, needs a credential and never crosses tenants', async () => {
  const n = await nodeIds();
  const q = { source: n.gw, target: n.orders, relation: 'calls' };
  assert.equal((await fetch(`${API}${ISO}/edges/traffic?${new URLSearchParams(q)}`)).status, 401);
  assert.equal((await traffic(ISO, admin, q)).status, 404, 'not a member of the isolation project');
  assert.equal((await traffic(ISO, iso, { ...q, target: n.gw })).status, 400);
  assert.equal((await traffic(ISO, iso, { ...q, relation: 'owns' })).status, 400);
  assert.equal((await traffic(ISO, iso, { ...q, source: 'x'.repeat(300) })).status, 400);
  // The same node IDs read from another tenant's environment see nothing.
  const other = await traffic(DEMO, admin, q);
  assert.equal(other.status, 200, JSON.stringify(other.body));
  assert.equal(other.body.total_calls, 0);
});
