import { test, afterEach } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { createServer } from 'node:http';

const require = createRequire(import.meta.url);
const { createFaults, readJson, sendJson, callJson, getJson, serve } = require('../src/common.cjs');

const saved = process.env.KARBIRD_DEMO_FAULTS;
afterEach(() => {
  if (saved === undefined) delete process.env.KARBIRD_DEMO_FAULTS;
  else process.env.KARBIRD_DEMO_FAULTS = saved;
});

async function listen(server) {
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  return {
    url: `http://127.0.0.1:${server.address().port}`,
    close: () => new Promise((r) => server.close(() => r())),
  };
}

async function faultServer(enabled) {
  if (enabled) process.env.KARBIRD_DEMO_FAULTS = '1';
  else delete process.env.KARBIRD_DEMO_FAULTS;
  const faults = createFaults();
  const srv = await listen(
    createServer(async (req, res) => {
      if (!(await faults.handle(req, res))) sendJson(res, 200, { passthrough: true });
    }),
  );
  return { faults, ...srv };
}

test('fault endpoint is hidden (404) unless KARBIRD_DEMO_FAULTS=1', async () => {
  const s = await faultServer(false);
  try {
    const r = await callJson('POST', `${s.url}/__demo/faults`, { latencyMs: 100, errorRate: 1 });
    assert.equal(r.status, 404);
    assert.equal(s.faults.enabled, false);
    assert.deepEqual(s.faults.state, { latencyMs: 0, errorRate: 0 });
    assert.equal(await s.faults.apply(), false);
  } finally {
    await s.close();
  }
});

test('fault endpoint reads and updates state when enabled', async () => {
  const s = await faultServer(true);
  try {
    assert.deepEqual((await getJson(`${s.url}/__demo/faults`)).body, { latencyMs: 0, errorRate: 0 });
    const r = await callJson('POST', `${s.url}/__demo/faults`, { latencyMs: 5, errorRate: 1 });
    assert.equal(r.status, 200);
    assert.deepEqual(r.body, { latencyMs: 5, errorRate: 1 });
    assert.equal(await s.faults.apply(), true);
    assert.equal((await getJson(`${s.url}/other`)).body.passthrough, true);
  } finally {
    await s.close();
  }
});

test('fault endpoint validates ranges and rejects bad input without changing state', async () => {
  const s = await faultServer(true);
  try {
    for (const body of [{ latencyMs: -1 }, { latencyMs: 30_001 }, { errorRate: 1.5 }, { errorRate: -0.1 }, { latencyMs: 'x' }]) {
      const r = await callJson('POST', `${s.url}/__demo/faults`, body);
      assert.equal(r.status, 400, JSON.stringify(body));
    }
    const bad = await fetch(`${s.url}/__demo/faults`, { method: 'POST', body: '{not json' });
    assert.equal(bad.status, 400);
    const put = await fetch(`${s.url}/__demo/faults`, { method: 'PUT' });
    assert.equal(put.status, 405);
    assert.deepEqual(s.faults.state, { latencyMs: 0, errorRate: 0 });
  } finally {
    await s.close();
  }
});

test('resources report this process CPU time and memory, even with faults disabled', async () => {
  const s = await faultServer(false);
  try {
    const r = await getJson(`${s.url}/__demo/resources`);
    assert.equal(r.status, 200);
    assert.equal(r.body.pid, process.pid);
    assert.ok(r.body.cpuMicros > 0 && r.body.rssBytes > 0, JSON.stringify(r.body));
    assert.equal(r.body.cpuHog, 0);
    assert.equal((await fetch(`${s.url}/__demo/resources`, { method: 'POST' })).status, 405);
  } finally {
    await s.close();
  }
});

test('cpu hog is hidden unless faults are enabled', async () => {
  const s = await faultServer(false);
  try {
    assert.equal((await callJson('POST', `${s.url}/__demo/cpu-hog`, { share: 0.5 })).status, 404);
    assert.equal(s.faults.hog.share, 0);
  } finally {
    await s.close();
  }
});

test('cpu hog burns the requested share of one core and stops at 0', async () => {
  const s = await faultServer(true);
  try {
    for (const body of [{ share: -0.1 }, { share: 1 }, { share: 'x' }]) {
      assert.equal((await callJson('POST', `${s.url}/__demo/cpu-hog`, body)).status, 400, JSON.stringify(body));
    }
    assert.equal(s.faults.hog.share, 0);
    const before = process.cpuUsage();
    const started = performance.now();
    assert.deepEqual((await callJson('POST', `${s.url}/__demo/cpu-hog`, { share: 0.6 })).body, { share: 0.6 });
    // The hog shares this event loop with the test, so wait with a timer that fires between bursts.
    await new Promise((r) => setTimeout(r, 600));
    const used = process.cpuUsage(before);
    const share = (used.user + used.system) / 1000 / (performance.now() - started);
    assert.ok(share > 0.35, `CPU share ${share.toFixed(2)} too low for a 0.6 hog`);
    assert.deepEqual((await getJson(`${s.url}/__demo/cpu-hog`)).body, { share: 0.6 });
    assert.deepEqual((await callJson('POST', `${s.url}/__demo/cpu-hog`, {})).body, { share: 0 });
    assert.equal(s.faults.hog.share, 0);
  } finally {
    s.faults.hog.set(0);
    await s.close();
  }
});

test('readJson enforces the size limit', async () => {
  const srv = await listen(
    createServer(async (req, res) => {
      try {
        sendJson(res, 200, await readJson(req, 16));
      } catch (e) {
        if (!res.headersSent && !res.destroyed) sendJson(res, 413, { error: e.message });
      }
    }),
  );
  try {
    const ok = await callJson('POST', srv.url, { a: 1 });
    assert.deepEqual(ok.body, { a: 1 });
    const empty = await callJson('POST', srv.url);
    assert.deepEqual(empty.body, {});
    await assert.rejects(callJson('POST', srv.url, { big: 'x'.repeat(1024) }));
  } finally {
    await srv.close();
  }
});

test('getJson keeps a short excerpt of non-JSON bodies', async () => {
  const srv = await listen(
    createServer((_req, res) => {
      res.writeHead(502);
      res.end('<html>' + 'x'.repeat(500));
    }),
  );
  try {
    const r = await getJson(srv.url);
    assert.equal(r.status, 502);
    assert.equal(r.body.raw.length, 200);
  } finally {
    await srv.close();
  }
});

test('serve() answers health, 404 for unknown routes and 500 on handler errors', async () => {
  delete process.env.KARBIRD_DEMO_FAULTS;
  const log = console.log;
  console.log = () => {};
  const server = serve('unit', 0, async (_req, res, url) => {
    if (url.pathname === '/boom') throw new TypeError('secret detail');
    if (url.pathname === '/hello') return void (res.writeHead(200), res.end('hi'));
    return false;
  });
  try {
    await new Promise((r) => (server.listening ? r() : server.once('listening', r)));
    const base = `http://127.0.0.1:${server.address().port}`;
    assert.deepEqual((await getJson(`${base}/health`)).body, { ok: true });
    assert.equal((await getJson(`${base}/nope`)).status, 404);
    assert.equal(await (await fetch(`${base}/hello`)).text(), 'hi');
    const boom = await getJson(`${base}/boom`);
    assert.equal(boom.status, 500);
    assert.deepEqual(boom.body, { error: 'internal error' });
    assert.equal((await getJson(`${base}/__demo/faults`)).status, 404);
  } finally {
    await new Promise((r) => server.close(() => r()));
    console.log = log;
  }
});
