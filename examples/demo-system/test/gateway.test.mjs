import { test } from 'node:test';
import assert from 'node:assert/strict';
import { captureServer, cleanEnv, freePort, src, startNode } from './helpers.mjs';

const reply = (req, res) => {
  res.writeHead(200, { 'content-type': 'application/json' });
  res.end(JSON.stringify({ id: 1 }));
};

async function gateway(env) {
  const port = await freePort();
  const gw = await startNode(src('gateway.cjs'), cleanEnv({ DEMO_GATEWAY_PORT: String(port), ...env }), /gateway listening/);
  return { ...gw, url: `http://127.0.0.1:${port}` };
}

const post = (url, body) =>
  fetch(url, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });

test('gateway spreads catalog reads round-robin over the replicas', async () => {
  const [a, b, auth] = await Promise.all([captureServer(reply), captureServer(reply), captureServer()]);
  const gw = await gateway({ DEMO_AUTH_URL: auth.url, DEMO_CATALOG_URLS: `${a.url}, ${b.url}` });
  try {
    for (let i = 0; i < 4; i++) {
      const r = await fetch(`${gw.url}/api/catalog/7`);
      assert.equal(r.status, 200);
    }
    assert.deepEqual([a.requests.length, b.requests.length], [2, 2]);
    assert.ok(a.requests.every((r) => r.url === '/items/7'));
  } finally {
    await gw.stop();
    await Promise.all([a.close(), b.close(), auth.close()]);
  }
});

test('the orchestrator can replace the replica list; reads follow the new list', async () => {
  const [a, b, auth] = await Promise.all([captureServer(reply), captureServer(reply), captureServer()]);
  const gw = await gateway({ KARBIRD_DEMO_FAULTS: '1', DEMO_AUTH_URL: auth.url, DEMO_CATALOG_URLS: `${a.url},${b.url}` });
  try {
    assert.deepEqual(await (await fetch(`${gw.url}/__demo/upstreams`)).json(), { catalog: [a.url, b.url] });
    const r = await post(`${gw.url}/__demo/upstreams`, { catalog: [b.url, b.url] });
    assert.equal(r.status, 200);
    assert.deepEqual(await r.json(), { catalog: [b.url] }, 'duplicates collapse');
    for (let i = 0; i < 3; i++) assert.equal((await fetch(`${gw.url}/api/catalog/1`)).status, 200);
    assert.deepEqual([a.requests.length, b.requests.length], [0, 3], 'a drained replica gets no requests');
  } finally {
    await gw.stop();
    await Promise.all([a.close(), b.close(), auth.close()]);
  }
});

test('replica lists are validated: loopback ports only, never empty', async () => {
  const gw = await gateway({ KARBIRD_DEMO_FAULTS: '1', DEMO_CATALOG_URLS: 'http://127.0.0.1:7006' });
  try {
    for (const catalog of [
      [],
      'http://127.0.0.1:7006',
      ['http://evil.example:80'],
      ['http://127.0.0.1:7006/path'],
      ['https://127.0.0.1:7006'],
      ['http://user@127.0.0.1:7006'],
      ['http://127.0.0.1:7006', 7016],
      Array.from({ length: 11 }, (_, i) => `http://127.0.0.1:${7100 + i}`),
    ]) {
      assert.equal((await post(`${gw.url}/__demo/upstreams`, { catalog })).status, 400, JSON.stringify(catalog));
    }
    assert.equal((await fetch(`${gw.url}/__demo/upstreams`, { method: 'PUT' })).status, 405);
    assert.deepEqual(await (await fetch(`${gw.url}/__demo/upstreams`)).json(), { catalog: ['http://127.0.0.1:7006'] });
  } finally {
    await gw.stop();
  }
});

test('the replica list and flags are hidden unless fault control is enabled', async () => {
  const gw = await gateway({ DEMO_CATALOG_URLS: 'http://127.0.0.1:7006' });
  try {
    assert.equal((await fetch(`${gw.url}/__demo/upstreams`)).status, 404);
    assert.equal((await post(`${gw.url}/__demo/upstreams`, { catalog: ['http://127.0.0.1:7016'] })).status, 404);
    assert.equal((await post(`${gw.url}/__demo/flags`, { flag: 'catalog-cache', enabled: true })).status, 404);
  } finally {
    await gw.stop();
  }
});

test('catalog-cache serves repeated reads from the gateway until it is switched off', async () => {
  const [catalog, auth] = await Promise.all([captureServer(reply), captureServer()]);
  const gw = await gateway({ KARBIRD_DEMO_FAULTS: '1', DEMO_AUTH_URL: auth.url, DEMO_CATALOG_URLS: catalog.url });
  try {
    const read = async () => assert.equal((await fetch(`${gw.url}/api/catalog/5`)).status, 200);
    await read();
    await read();
    assert.equal(catalog.requests.length, 2, 'no cache while the flag is off');

    assert.equal((await post(`${gw.url}/__demo/flags`, { flag: 'catalog-cache', enabled: true })).status, 200);
    await read();
    await read();
    await read();
    assert.equal(catalog.requests.length, 3, 'one upstream read, then cached');

    await post(`${gw.url}/__demo/flags`, { flag: 'catalog-cache', enabled: false });
    await read();
    assert.equal(catalog.requests.length, 4, 'switching off drops the cache');
    assert.equal(auth.requests.length, 6, 'auth is still checked on every request');
  } finally {
    await gw.stop();
    await Promise.all([catalog.close(), auth.close()]);
  }
});
