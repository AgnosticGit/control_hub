import { test } from 'node:test';
import assert from 'node:assert/strict';
import { setTimeout as sleep } from 'node:timers/promises';
import { captureServer, cleanEnv, src, startNode } from './helpers.mjs';

const loadgen = src('loadgen.mjs');

async function announcedRps(rps) {
  const cap = await captureServer();
  const env = cleanEnv({ DEMO_GATEWAY_URL: cap.url, ...(rps === undefined ? {} : { DEMO_RPS: rps }) });
  const p = await startNode(loadgen, env, /load generator: .* req\/s/);
  await p.stop();
  await cap.close();
  return /load generator: ([\d.]+) req\/s/.exec(p.output())?.[1];
}

test('clamps the request rate to 0.2..200 and ignores non-numeric values', async () => {
  assert.equal(await announcedRps(undefined), '5');
  assert.equal(await announcedRps('12'), '12');
  assert.equal(await announcedRps('100000'), '200');
  assert.equal(await announcedRps('0'), '0.2');
  assert.equal(await announcedRps('-3'), '0.2');
  assert.equal(await announcedRps('fast'), '5');
});

test('sends a mix of catalog reads, order reads and order writes to the gateway only', async () => {
  const cap = await captureServer((req, res) => {
    res.writeHead(req.url === '/api/orders/1' ? 500 : 200);
    res.end();
  });
  const p = await startNode(loadgen, cleanEnv({ DEMO_GATEWAY_URL: cap.url, DEMO_RPS: '200' }), /load generator/);
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
