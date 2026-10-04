import { test } from 'node:test';
import assert from 'node:assert/strict';
import { captureServer, cleanEnv, freePort, src, startNode } from './helpers.mjs';

test('gateway spreads catalog reads round-robin over the replicas', async () => {
  const reply = (req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' });
    res.end(JSON.stringify({ id: 1 }));
  };
  const [a, b, auth] = await Promise.all([captureServer(reply), captureServer(reply), captureServer()]);
  const port = await freePort();
  const gw = await startNode(
    src('gateway.cjs'),
    cleanEnv({ DEMO_GATEWAY_PORT: String(port), DEMO_AUTH_URL: auth.url, DEMO_CATALOG_URLS: `${a.url}, ${b.url}` }),
    /gateway listening/,
  );
  try {
    for (let i = 0; i < 4; i++) {
      const r = await fetch(`http://127.0.0.1:${port}/api/catalog/7`);
      assert.equal(r.status, 200);
    }
    assert.deepEqual([a.requests.length, b.requests.length], [2, 2]);
    assert.ok(a.requests.every((r) => r.url === '/items/7'));
  } finally {
    await gw.stop();
    await Promise.all([a.close(), b.close(), auth.close()]);
  }
});
