// Infrastructure layer end to end: OTLP metrics (Karbird agent) and spans (SDK) go through
// ingest -> Kafka -> processor -> ClickHouse and come back from GET …/infrastructure as
// machines, containers, matched instances and a noisy-neighbour hint. Writes only into the
// isolation tenant with unique names, so reruns and the demo map are unaffected.
// Run: node --test "tests/integration/*.test.mjs"
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { API, DEMO, ISO, Session, decodeFields, encodeMetrics, encodeTraces, eventually, hex, ingestMetrics, nowNs, otlpHttp } from './lib.mjs';

const MIB = 1024 * 1024;
const SEC = 1_000_000_000n;
const x = hex(4);
const HOST = `it-vps-${x}`;
const ORDERS = `it-orders-${x}`;
const INVENTORY = `it-inventory-${x}`;
const ORDERS_CTR = hex(32);
const INVENTORY_CTR = hex(32);
const AT = nowNs();
const T1 = AT - 90n * SEC;
const T2 = AT - 10n * SEC;

let iso;
let admin;
before(async () => {
  iso = new Session();
  await iso.login('isolation@karbird.local');
  admin = new Session();
  await admin.login('admin@karbird.local');
});

const cumulative = (name, unit, values) => ({
  name,
  unit,
  kind: 'sum',
  monotonic: true,
  points: values.flatMap(([attributes, v1, v2]) => [
    { attributes, value: v1, startNs: T1 - SEC, timeNs: T1 },
    { attributes, value: v2, startNs: T1 - SEC, timeNs: T2 },
  ]),
});
const gauge = (name, unit, points) => ({ name, unit, kind: 'gauge', points: points.map(([attributes, value]) => ({ attributes, value, timeNs: T2 })) });

/** vps-app: 1 core at 95 % busy, 870/1024 MB; inventory uses 0.95 cores, orders 0.02 (80 s apart). */
function agentExport() {
  const container = (id, name, cores) => ({
    attributes: { 'host.name': HOST, 'host.id': HOST, 'container.id': id, 'container.name': `/${name}` },
    metrics: [
      cumulative('container.cpu.usage.total', 'ns', [[{}, 1e12, 1e12 + cores * 80e9]]),
      gauge('container.memory.usage.total', 'By', [[{}, 200 * MIB]]),
    ],
  });
  return encodeMetrics([
    {
      attributes: { 'host.name': HOST, 'host.id': HOST },
      metrics: [
        cumulative('system.cpu.time', 's', [
          [{ cpu: 'cpu0', state: 'user' }, 1000, 1000 + 76],
          [{ cpu: 'cpu0', state: 'idle' }, 1000, 1000 + 4],
        ]),
        gauge('system.cpu.logical.count', '{cpu}', [[{}, 1]]),
        { ...cumulative('system.memory.usage', 'By', [[{ state: 'used' }, 870 * MIB, 870 * MIB], [{ state: 'free' }, 154 * MIB, 154 * MIB]]), monotonic: false },
        gauge('system.memory.limit', 'By', [[{}, 1024 * MIB]]),
        { ...cumulative('system.filesystem.usage', 'By', [[{ mountpoint: '/', state: 'used' }, 9e9, 9e9], [{ mountpoint: '/', state: 'free' }, 16e9, 16e9]]), monotonic: false },
        cumulative('system.network.io', 'By', [[{ device: 'eth0', direction: 'receive' }, 0, 80 * 2048], [{ device: 'lo', direction: 'receive' }, 0, 8e9]]),
      ],
    },
    container(ORDERS_CTR, ORDERS, 0.02),
    container(INVENTORY_CTR, INVENTORY, 0.95),
  ]);
}

/** `n` server spans of `ms` each, starting at `fromNs`, one per second. */
function spans(n, fromNs, ms) {
  return Array.from({ length: n }, (_, i) => {
    const start = fromNs + BigInt(i) * SEC;
    return { traceId: hex(16), spanId: hex(8), name: 'GET /x', kind: 2, startNs: start, endNs: start + BigInt(ms) * 1_000_000n };
  });
}

async function snapshot(scope = ISO, session = iso, at = AT) {
  return session.json(`${scope}/infrastructure?at=${at}`);
}

test('agent metrics and SDK spans become a machine, containers, instances and a noisy-neighbour hint', async () => {
  const before = await ingestMetrics();
  const m = await otlpHttp('metrics', agentExport());
  assert.equal(m.status, 200);
  assert.equal(m.body.length, 0, 'no partial_success: every point accepted');
  const after = await ingestMetrics();
  assert.ok(after['karbird_ingest_items_total{signal="metrics",result="accepted"}'] > before['karbird_ingest_items_total{signal="metrics",result="accepted"}']);

  const ordersRes = { 'service.instance.id': `${ORDERS}-1`, 'container.id': ORDERS_CTR };
  // Baseline: 50 ms 20 minutes ago; now 400 ms.
  assert.equal((await otlpHttp('traces', encodeTraces(ORDERS, spans(25, AT - 1200n * SEC, 50), ordersRes))).status, 200);
  assert.equal((await otlpHttp('traces', encodeTraces(ORDERS, spans(25, AT - 120n * SEC, 400), ordersRes))).status, 200);
  const invRes = { 'service.instance.id': `${INVENTORY}-1`, 'host.name': INVENTORY_CTR.slice(0, 12) };
  assert.equal((await otlpHttp('traces', encodeTraces(INVENTORY, spans(25, AT - 120n * SEC, 5), invRes))).status, 200);

  const snap = await eventually(
    async () => {
      const r = await snapshot();
      assert.equal(r.status, 200, JSON.stringify(r.body));
      assert.ok(r.body.hints.some((h) => h.service_name === ORDERS), JSON.stringify(r.body.hints));
      return r.body;
    },
    { timeoutMs: 60_000, intervalMs: 2000 },
  );

  const host = snap.hosts.find((h) => h.id === HOST);
  assert.ok(host, JSON.stringify(snap.hosts.map((h) => h.id)));
  assert.equal(host.has_metrics, true);
  assert.ok(Math.abs(host.cpu_utilization - 0.95) < 0.01, `cpu ${host.cpu_utilization}`);
  assert.equal(host.cpu_cores, 1);
  assert.equal(Math.round(host.memory_used_bytes / MIB), 870);
  assert.equal(Math.round(host.memory_total_bytes / MIB), 1024);
  assert.equal(host.disk_total_bytes, 25e9);
  assert.ok(Math.abs(host.network_rx_bytes_per_sec - 2048) < 1, 'loopback is not network traffic');

  const inv = host.containers.find((c) => c.id === INVENTORY_CTR);
  assert.equal(inv.name, INVENTORY, 'leading slash of the Docker name is dropped');
  assert.ok(Math.abs(inv.cpu_share - 0.95) < 0.01);
  assert.equal(inv.service_name, INVENTORY);
  const ord = host.containers.find((c) => c.id === ORDERS_CTR);
  assert.ok(Math.abs(ord.cpu_share - 0.02) < 0.01);

  const ordersInstance = snap.instances.find((i) => i.service_name === ORDERS);
  assert.deepEqual([ordersInstance.match_reason, ordersInstance.host_id, ordersInstance.container_id], ['container_id', HOST, ORDERS_CTR]);
  assert.ok(ordersInstance.p95_ms >= 300 && ordersInstance.baseline_p95_ms <= 60, JSON.stringify(ordersInstance));
  const invInstance = snap.instances.find((i) => i.service_name === INVENTORY);
  assert.deepEqual([invInstance.match_reason, invInstance.container_id], ['container_hostname', INVENTORY_CTR], 'short ID as host.name');

  const hint = snap.hints.find((h) => h.service_name === ORDERS);
  assert.equal(hint.neighbor_name, INVENTORY);
  assert.equal(hint.host_id, HOST);
  assert.ok(hint.neighbor_cpu_share >= 0.9);
  assert.match(hint.reason, /p95/);
  assert.ok(!snap.hints.some((h) => h.service_name === INVENTORY), 'inventory is not its own victim');
});

test('a machine at another moment has no metrics and no hint', async () => {
  const r = await snapshot(ISO, iso, AT - 3600n * SEC);
  assert.equal(r.status, 200);
  assert.ok(!r.body.hosts.some((h) => h.id === HOST));
  assert.ok(!r.body.hints.some((h) => h.service_name === ORDERS));
});

test('histograms are rejected per point and reported in partial_success', async () => {
  const body = encodeMetrics([
    {
      attributes: { 'host.name': `it-hist-${x}` },
      metrics: [
        { name: 'http.server.duration', kind: 'histogram', points: [{ value: 12, timeNs: T2 }] },
        gauge('system.cpu.logical.count', '{cpu}', [[{}, 2]]),
      ],
    },
  ]);
  const r = await otlpHttp('metrics', body);
  assert.equal(r.status, 200);
  const partial = decodeFields(decodeFields(r.body)[1][0]);
  assert.equal(partial[1][0], 1n, 'one rejected data point');
  assert.match(partial[2][0].toString(), /unsupported/);
});

test('the infrastructure snapshot needs a credential and is scoped to the tenant', async () => {
  assert.equal((await fetch(`${API}${ISO}/infrastructure`)).status, 401);
  assert.equal((await snapshot(ISO, admin)).status, 404, 'demo admin cannot read the isolation tenant');
  assert.equal((await snapshot(DEMO, iso)).status, 404, 'isolation user cannot read the demo tenant');
  const demo = await snapshot(DEMO, admin);
  assert.equal(demo.status, 200);
  assert.ok(!demo.body.hosts.some((h) => h.id === HOST), 'isolation machines never show up in the demo tenant');
  assert.ok(!demo.body.instances.some((i) => i.service_name === ORDERS));
});

test('a malformed time is a 400', async () => {
  assert.equal((await iso.json(`${ISO}/infrastructure?at=yesterday`)).status, 400);
});
