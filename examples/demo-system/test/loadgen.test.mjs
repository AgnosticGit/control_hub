import { test } from 'node:test';
import assert from 'node:assert/strict';
import { setTimeout as sleep } from 'node:timers/promises';
import { captureServer, cleanEnv, src, startNode } from './helpers.mjs';

const loadgen = src('loadgen.mjs');

async function announcedRps(rps) {
  const cap = await captureServer();
  const env = cleanEnv({ DEMO_GATEWAY_URL: cap.url, DEMO_LOAD_CONTROL_PORT: '0', ...(rps === undefined ? {} : { DEMO_RPS: rps }) });
  const p = await startNode(loadgen, env, /load generator: .* req\/s/);
  await p.stop();
  await cap.close();
  return /load generator: ([\d.]+) req\/s/.exec(p.output())?.[1];
}

test('defaults to ten requests per minute and bounds overrides', async () => {
  assert.ok(Math.abs(Number(await announcedRps(undefined)) - 1 / 6) < 0.000001);
  assert.equal(await announcedRps('12'), '12');
  assert.equal(await announcedRps('100000'), '200');
  assert.equal(await announcedRps('0'), String(1 / 60));
  assert.equal(await announcedRps('-3'), String(1 / 60));
  assert.ok(Math.abs(Number(await announcedRps('fast')) - 1 / 6) < 0.000001);
});

test('sends a mix of catalog reads, order reads and order writes to the gateway only', async () => {
  const cap = await captureServer((req, res) => {
    res.writeHead(req.url === '/api/orders/1' ? 500 : 200);
    res.end();
  });
  const p = await startNode(loadgen, cleanEnv({ DEMO_GATEWAY_URL: cap.url, DEMO_LOAD_CONTROL_PORT: '0', DEMO_RPS: '200' }), /load generator/);
  await sleep(1500);
  await p.stop();
  await cap.close();

  assert.ok(cap.requests.length >= 50, `only ${cap.requests.length} requests`);
  const kinds = new Set();
  for (const r of cap.requests) {
    if (r.method === 'POST') {
      assert.equal(r.url, '/api/orders');
      kinds.add('write');
    } else {
      assert.equal(r.method, 'GET');
      const m = /^\/api\/(catalog|orders)\/(\d+)$/.exec(r.url);
      assert.ok(m, `unexpected path ${r.url}`);
      const id = Number(m[2]);
      assert.ok(id >= 1 && id <= (m[1] === 'catalog' ? 60 : 55), `id out of range: ${r.url}`);
      kinds.add(m[1]);
    }
    assert.equal(r.body, '');
    assert.equal(r.headers.authorization, undefined);
  }
  assert.deepEqual([...kinds].sort(), ['catalog', 'orders', 'write']);
});

test('incident probes replace scheduled requests within the configured rate', async () => {
  const gateway = await captureServer();
  const orders = await captureServer();
  const p = await startNode(loadgen, cleanEnv({
    DEMO_GATEWAY_URL: gateway.url,
    DEMO_ORDERS_URL: orders.url,
    DEMO_LOAD_CONTROL_PORT: '0',
    DEMO_RPS: '40',
  }), /load control: \d+/);
  const port = Number(/load control: (\d+)/.exec(p.output())?.[1]);
  const response = await fetch(`http://127.0.0.1:${port}/__demo/incident-state`, {
    method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ active: true }),
  });
  assert.equal(response.status, 204);
  await sleep(300);
  await p.stop();
  await gateway.close();
  await orders.close();
  assert.ok(orders.requests.length > 0);
  assert.ok(orders.requests.every((request) => request.url === '/__demo/incident-probe'));
  assert.ok(orders.requests.length + gateway.requests.length <= 20, 'probes must use scheduled load slots');
});
