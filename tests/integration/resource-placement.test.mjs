// Databases on our own machines are placed automatically (ADR-0018): the Karbird Collector reports
// each container's image and Docker Compose service, the SDK's client spans report the address
// they connect to, and GET …/infrastructure says where each database runs. A managed database
// (an address no agent answers to) is external. Writes only into the isolation tenant with
// unique names, so reruns and the demo map are unaffected.
// Run: node --test "tests/integration/*.test.mjs"
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { API, DEMO, ISO, Session, encodeMetrics, encodeTraces, eventually, hex, nowNs, otlpHttp } from './lib.mjs';

const MIB = 1024 * 1024;
const SEC = 1_000_000_000n;
const x = hex(4);
const HOST = `it-rp-${x}`;
const ORDERS = `it-rp-orders-${x}`;
const ORDERS_CTR = hex(32);
const PG_CTR = hex(32);
const COMPOSE = `pg-${x}`;
const DB_COMPOSE = `orders_${x}`;
const DB_LOCAL = `stock_${x}`;
const DB_MANAGED = `billing_${x}`;
const MANAGED = `rc1a-${x}.mdb.yandexcloud.net`;
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

const cpu = (cores) => ({
  name: 'container.cpu.usage.total',
  unit: 'ns',
  kind: 'sum',
  monotonic: true,
  points: [
    { attributes: {}, value: 1e12, startNs: T1 - SEC, timeNs: T1 },
    { attributes: {}, value: 1e12 + cores * 80e9, startNs: T1 - SEC, timeNs: T2 },
  ],
});
const rss = (mb) => ({ name: 'container.memory.usage.total', unit: 'By', kind: 'gauge', points: [{ attributes: {}, value: mb * MIB, timeNs: T2 }] });

function agentExport() {
  return encodeMetrics([
    {
      attributes: { 'host.name': HOST, 'host.id': HOST },
      metrics: [{ name: 'system.cpu.logical.count', unit: '{cpu}', kind: 'gauge', points: [{ attributes: {}, value: 2, timeNs: T2 }] }],
    },
    {
      attributes: { 'host.name': HOST, 'host.id': HOST, 'container.id': ORDERS_CTR, 'container.name': `/${ORDERS}`, 'container.image.name': 'ghcr.io/acme/orders:1.4.2' },
      metrics: [cpu(0.1), rss(120)],
    },
    {
      attributes: {
        'host.name': HOST,
        'host.id': HOST,
        'container.id': PG_CTR,
        'container.name': `/demo-${COMPOSE}-1`,
        'container.image.name': 'postgres:16-alpine',
        'docker.compose.service': COMPOSE,
      },
      metrics: [cpu(0.04), rss(96)],
    },
  ]);
}

/** Client spans from orders to a PostgreSQL database at `address`. */
function dbCalls(db, address) {
  return Array.from({ length: 3 }, (_, i) => {
    const start = AT - 60n * SEC + BigInt(i) * SEC;
    return {
      traceId: hex(16),
      spanId: hex(8),
      name: 'SELECT',
      kind: 3,
      startNs: start,
      endNs: start + 2_000_000n,
      attributes: { 'db.system': 'postgresql', 'db.name': db, 'server.address': address, 'server.port': '5432' },
    };
  });
}

const snapshot = (scope = ISO, session = iso) => session.json(`${scope}/infrastructure?at=${AT}`);
const byName = (snap, db) => snap.resources.find((r) => r.name === `postgresql: ${db}`);

test('a database in a container on our machine is internal; a managed one is external', async () => {
  assert.equal((await otlpHttp('metrics', agentExport())).status, 200);
  const res = { 'service.instance.id': `${ORDERS}-1`, 'container.id': ORDERS_CTR };
  const calls = [...dbCalls(DB_COMPOSE, COMPOSE), ...dbCalls(DB_LOCAL, 'localhost'), ...dbCalls(DB_MANAGED, MANAGED)];
  assert.equal((await otlpHttp('traces', encodeTraces(ORDERS, calls, res))).status, 200);

  const snap = await eventually(
    async () => {
      const r = await snapshot();
      assert.equal(r.status, 200, JSON.stringify(r.body));
      for (const db of [DB_COMPOSE, DB_LOCAL, DB_MANAGED]) assert.ok(byName(r.body, db), `${db} in ${JSON.stringify(r.body.resources)}`);
      assert.ok(r.body.hosts.some((h) => h.id === HOST && h.containers.some((c) => c.id === PG_CTR)));
      return r.body;
    },
    { timeoutMs: 60_000, intervalMs: 2000 },
  );

  const compose = byName(snap, DB_COMPOSE);
  assert.equal(compose.kind, 'database');
  assert.deepEqual([compose.location, compose.host_id, compose.container_id, compose.match_reason], ['internal', HOST, PG_CTR, 'compose_service']);
  assert.deepEqual(compose.addresses, [`${COMPOSE}:5432`]);

  const local = byName(snap, DB_LOCAL);
  assert.deepEqual([local.location, local.host_id, local.container_id, local.match_reason], ['internal', HOST, PG_CTR, 'loopback']);

  const managed = byName(snap, DB_MANAGED);
  assert.deepEqual([managed.location, managed.host_id, managed.container_id], ['external', null, null]);
  assert.match(managed.reason, /managed or external database/);

  const pg = snap.hosts.find((h) => h.id === HOST).containers.find((c) => c.id === PG_CTR);
  assert.equal(pg.image, 'postgres:16-alpine');
  assert.equal(pg.compose_service, COMPOSE);
  assert.ok([compose.node_id, local.node_id].includes(pg.resource_id), JSON.stringify(pg));
  const orders = snap.hosts.find((h) => h.id === HOST).containers.find((c) => c.id === ORDERS_CTR);
  assert.equal(orders.resource_id, null, 'the caller container runs no database');
});

test('resource placement needs a credential and never leaks across tenants', async () => {
  assert.equal((await fetch(`${API}${ISO}/infrastructure?at=${AT}`)).status, 401);
  assert.equal((await snapshot(ISO, admin)).status, 404);
  const demo = await snapshot(DEMO, admin);
  assert.equal(demo.status, 200);
  for (const db of [DB_COMPOSE, DB_LOCAL, DB_MANAGED]) assert.equal(byName(demo.body, db), undefined, `${db} not in the demo tenant`);
});
