// Machine timeline end to end: agent-style OTLP metrics for one machine and four containers go
// through ingest -> Kafka -> processor -> ClickHouse and come back from
// GET …/infrastructure/timeline as CPU and memory per bucket plus container events derived from
// the metrics (restart, out of memory, redeploy, start). Writes only into the isolation tenant
// with unique names, so reruns and the demo are unaffected.
// Run: node --test "tests/integration/*.test.mjs"
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { API, DEMO, ISO, Session, encodeMetrics, eventually, hex, nowNs, otlpHttp } from './lib.mjs';

const MIB = 1024 * 1024;
const SEC = 1_000_000_000n;
const x = hex(4);
const HOST = `it-tl-vps-${x}`;
const AT = nowNs();
const FROM = AT - 600n * SEC;
/** Points every 20 s from FROM: index 0..29, the last 20 s before AT. */
const at = (i) => FROM + BigInt(i) * 20n * SEC;
const range = (a, b) => Array.from({ length: b - a + 1 }, (_, k) => a + k);

const WEB = { id: hex(32), name: `it-web-${x}` };
const HOG = { id: hex(32), name: `it-hog-${x}` };
const WORKER_OLD = { id: hex(32), name: `it-worker-${x}` };
const WORKER_NEW = { id: hex(32), name: WORKER_OLD.name };
const CRON = { id: hex(32), name: `it-cron-${x}` };

let iso;
let admin;
before(async () => {
  iso = new Session();
  await iso.login('isolation@karbird.local');
  admin = new Session();
  await admin.login('admin@karbird.local');
});

const sum = (name, unit, monotonic, points) => ({ name, unit, kind: 'sum', monotonic, points: points.map((p) => ({ startNs: FROM - SEC, ...p })) });
const gauge = (name, unit, points) => ({ name, unit, kind: 'gauge', points });

/** A container reporting at point indexes `idx`; `cpu(i)` is its cumulative CPU counter, `mem(i)` its memory. */
function container(c, idx, cpu, mem, limit = 256 * MIB) {
  return {
    attributes: { 'host.name': HOST, 'host.id': HOST, 'container.id': c.id, 'container.name': `/${c.name}` },
    metrics: [
      sum('container.cpu.usage.total', 'ns', true, idx.map((i) => ({ value: cpu(i), timeNs: at(i) }))),
      gauge('container.memory.usage.total', 'By', idx.map((i) => ({ value: mem(i), timeNs: at(i) }))),
      gauge('container.memory.usage.limit', 'By', idx.map((i) => ({ value: limit, timeNs: at(i) }))),
    ],
  };
}

/**
 * Machine at 50 % CPU and 512/1024 MB the whole 10 minutes. web restarts at point 15 (its CPU
 * counter starts over), hog stops at point 20 with memory at 99 % of its limit, worker is replaced
 * by a new container at point 15, cron starts at point 25.
 */
function agentExport() {
  const all = range(0, 29);
  return encodeMetrics([
    {
      attributes: { 'host.name': HOST, 'host.id': HOST },
      metrics: [
        sum('system.cpu.time', 's', true, [
          ...all.map((i) => ({ attributes: { cpu: 'cpu0', state: 'user' }, value: 1000 + i * 10, timeNs: at(i) })),
          ...all.map((i) => ({ attributes: { cpu: 'cpu0', state: 'idle' }, value: 1000 + i * 10, timeNs: at(i) })),
        ]),
        sum('system.memory.usage', 'By', false, [
          ...all.map((i) => ({ attributes: { state: 'used' }, value: 512 * MIB, timeNs: at(i) })),
          ...all.map((i) => ({ attributes: { state: 'free' }, value: 512 * MIB, timeNs: at(i) })),
        ]),
        gauge('system.memory.limit', 'By', all.map((i) => ({ value: 1024 * MIB, timeNs: at(i) }))),
      ],
    },
    container(WEB, all, (i) => (i < 15 ? 1e12 + i * 1e9 : (i - 15) * 1e9), () => 64 * MIB),
    container(HOG, range(0, 20), (i) => 1e12 + i * 1e9, (i) => (i < 20 ? 128 * MIB : 0.99 * 256 * MIB)),
    container(WORKER_OLD, range(0, 13), (i) => 1e12 + i * 1e9, () => 32 * MIB),
    container(WORKER_NEW, range(15, 29), (i) => i * 1e9, () => 32 * MIB),
    container(CRON, range(25, 29), (i) => i * 1e9, () => 16 * MIB),
  ]);
}

const timeline = (query, scope = ISO, session = iso) => session.json(`${scope}/infrastructure/timeline?${query}`);
const WINDOW = `from=${FROM}&to=${AT}&buckets=60`;

test('collector metrics become machine CPU and memory over time and container events', async () => {
  const r = await otlpHttp('metrics', agentExport());
  assert.equal(r.status, 200);
  assert.equal(r.body.length, 0, 'no partial_success: every point accepted');

  const t = await eventually(
    async () => {
      const res = await timeline(WINDOW);
      assert.equal(res.status, 200, JSON.stringify(res.body));
      const mine = res.body.events.filter((e) => e.host_id === HOST);
      assert.equal(mine.length, 4, JSON.stringify(mine));
      return res.body;
    },
    { timeoutMs: 60_000, intervalMs: 2000 },
  );

  assert.equal(t.buckets, 60);
  assert.equal(t.bucket_ns, (10n * SEC).toString());
  assert.equal(t.from_ns, FROM.toString());

  const host = t.hosts.find((h) => h.id === HOST);
  assert.ok(host, JSON.stringify(t.hosts.map((h) => h.id)));
  assert.equal(host.name, HOST);
  assert.equal(host.cpu.length, 60);
  assert.equal(host.memory.length, 60);
  const cpu = host.cpu.filter((v) => v !== null);
  const memory = host.memory.filter((v) => v !== null);
  assert.ok(cpu.length >= 25, `CPU in most buckets: ${JSON.stringify(host.cpu)}`);
  for (const v of cpu) assert.ok(Math.abs(v - 0.5) < 0.01, `cpu ${v}`);
  assert.ok(memory.length >= 25);
  for (const v of memory) assert.ok(Math.abs(v - 0.5) < 0.01, `memory ${v}`);
  // Nothing reported in the last 20 s: no data, not zero.
  assert.equal(host.cpu[59], null);

  const events = t.events.filter((e) => e.host_id === HOST);
  const byName = (name) => events.filter((e) => e.container_name === name);
  const times = events.map((e) => BigInt(e.time_ns));
  assert.deepEqual(times, [...times].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0)), 'oldest first');
  for (const e of events) assert.equal(e.host_name, HOST);

  const [restart] = byName(WEB.name);
  assert.deepEqual([restart.kind, restart.time_ns, restart.container_id], ['restarted', at(15).toString(), WEB.id]);
  assert.equal(restart.detail, `${WEB.name} restarted on ${HOST}`);

  const [oom] = byName(HOG.name);
  assert.deepEqual([oom.kind, oom.time_ns], ['out_of_memory', at(20).toString()]);
  assert.match(oom.detail, /stopped with memory at 99% of its limit: probably out of memory/);

  const worker = byName(WORKER_OLD.name);
  assert.equal(worker.length, 1, 'stop and start of the same name are one redeploy');
  assert.deepEqual([worker[0].kind, worker[0].time_ns, worker[0].container_id], ['redeployed', at(15).toString(), WORKER_NEW.id]);
  assert.equal(worker[0].detail, `${WORKER_OLD.name} redeployed on ${HOST}: container ${WORKER_NEW.id.slice(0, 12)} replaced ${WORKER_OLD.id.slice(0, 12)}`);

  const [start] = byName(CRON.name);
  assert.deepEqual([start.kind, start.time_ns], ['started', at(25).toString()]);
});

test('a window before the machine reported has neither its track nor its events', async () => {
  const r = await timeline(`from=${FROM - 3600n * SEC}&to=${FROM - 1200n * SEC}`);
  assert.equal(r.status, 200);
  assert.ok(!r.body.hosts.some((h) => h.id === HOST));
  assert.ok(!r.body.events.some((e) => e.host_id === HOST));
});

test('the bucket count is bounded', async () => {
  const r = await timeline(`from=${FROM}&to=${AT}&buckets=50000`);
  assert.equal(r.status, 200);
  assert.equal(r.body.buckets, 1000);
});

test('the machine timeline needs a credential and is scoped to the tenant', async () => {
  assert.equal((await fetch(`${API}${ISO}/infrastructure/timeline?${WINDOW}`)).status, 401);
  assert.equal((await timeline(WINDOW, ISO, admin)).status, 404, 'demo admin cannot read the isolation tenant');
  assert.equal((await timeline(WINDOW, DEMO, iso)).status, 404, 'isolation user cannot read the demo tenant');
  const demo = await timeline(WINDOW, DEMO, admin);
  assert.equal(demo.status, 200);
  assert.ok(!demo.body.hosts.some((h) => h.id === HOST), 'isolation machines never show up in the demo tenant');
  assert.ok(!demo.body.events.some((e) => e.host_id === HOST));
});

test('a malformed or too long range is a 400', async () => {
  assert.equal((await timeline('from=yesterday')).status, 400);
  assert.equal((await timeline(`from=${AT}&to=${FROM}`)).status, 400);
  assert.equal((await timeline(`from=${AT - 8n * 24n * 3600n * SEC}&to=${AT}`)).status, 400);
});
