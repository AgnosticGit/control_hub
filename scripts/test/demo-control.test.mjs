import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createDemoControl, listenControl } from '../demo-control.mjs';

async function freePort() {
  const s = createServer();
  await new Promise((r) => s.listen(0, '127.0.0.1', r));
  const { port } = s.address();
  await new Promise((r) => s.close(() => r()));
  return port;
}

const NAMES = ['gateway', 'orders', 'catalog-1', 'catalog-2', 'catalog-3'];

/**
 * Stands in for `supervise()`: "starting" an instance listens on its port with /health, and
 * the gateway also keeps a replica list. `broken` names never become healthy.
 */
async function fakeDemo({ broken = new Set() } = {}) {
  const ports = Object.fromEntries(await Promise.all(NAMES.map(async (n) => [n, await freePort()])));
  const url = (n) => `http://127.0.0.1:${ports[n]}`;
  const configured = ['catalog-1', 'catalog-2', 'catalog-3'].map(url);
  const live = new Map();
  const starts = [];
  const routed = [];
  let upstreams = configured;
  let nextPid = 100;

  const handler = (name) => (req, res) => {
    const send = (status, body) => {
      res.writeHead(status, { 'content-type': 'application/json' });
      res.end(JSON.stringify(body));
    };
    if (req.url === '/health') return send(200, { ok: true });
    if (name === 'gateway' && req.url === '/__demo/upstreams') {
      if (req.method === 'GET') return send(200, { catalog: upstreams });
      const chunks = [];
      req.on('data', (c) => chunks.push(c));
      req.on('end', () => {
        upstreams = JSON.parse(Buffer.concat(chunks).toString()).catalog;
        routed.push(upstreams);
        send(200, { catalog: upstreams });
      });
      return;
    }
    send(404, {});
  };

  const supervisor = {
    start(p) {
      if (live.has(p.name)) throw new Error(`${p.name} is already running`);
      const entry = { pid: nextPid++, server: null };
      if (!broken.has(p.name)) {
        entry.server = createServer(handler(p.name));
        entry.server.listen(ports[p.name], '127.0.0.1');
      }
      if (p.name === 'gateway') upstreams = configured;
      live.set(p.name, entry);
      starts.push({ name: p.name, env: p.env });
    },
    stop(name) {
      const e = live.get(name);
      if (!e) return Promise.resolve(false);
      live.delete(name);
      if (!e.server) return Promise.resolve(true);
      e.server.closeAllConnections();
      return new Promise((r) => e.server.close(() => r(true)));
    },
    running: (name) => live.has(name),
    pid: (name) => live.get(name)?.pid ?? null,
  };

  const proc = (name) => ({ name, cmd: 'node', args: [], env: { KARBIRD_SERVICE_VERSION: '0.1.0', KARBIRD_SERVICE_NAME: name } });
  const instances = new Map(
    NAMES.map((n) => [n, { service: n.startsWith('catalog') ? 'catalog' : n, port: ports[n], proc: proc(n) }]),
  );
  for (const n of NAMES) supervisor.start(proc(n));
  starts.length = 0;

  const control = createDemoControl({
    supervisor,
    instances,
    scalable: { catalog: { replicas: ['catalog-1', 'catalog-2', 'catalog-3'], router: 'gateway' } },
    releases: { orders: { instance: 'orders', versions: ['0.1.0', '0.2.0'], version: '0.1.0' } },
    healthTimeoutMs: 1500,
    pollMs: 20,
  });
  return {
    control,
    supervisor,
    url,
    starts,
    routed,
    upstreams: () => upstreams,
    close: async () => {
      for (const n of [...live.keys()]) await supervisor.stop(n);
    },
  };
}

test('restart replaces the process and keeps a replica out of rotation while it restarts', async () => {
  const d = await fakeDemo();
  try {
    const before = d.supervisor.pid('catalog-2');
    const out = await d.control.restart('catalog-2');
    assert.equal(out.instance, 'catalog-2');
    assert.equal(out.service, 'catalog');
    assert.equal(out.previous_pid, before);
    assert.notEqual(out.pid, before);
    assert.equal(out.pid, d.supervisor.pid('catalog-2'));
    assert.ok(out.healthy_after_ms >= 0);
    assert.deepEqual(d.routed, [
      [d.url('catalog-1'), d.url('catalog-3')],
      [d.url('catalog-1'), d.url('catalog-2'), d.url('catalog-3')],
    ]);
    assert.deepEqual(d.starts.map((s) => s.name), ['catalog-2']);
  } finally {
    await d.close();
  }
});

test('restart refuses unknown and stopped instances', async () => {
  const d = await fakeDemo();
  try {
    await assert.rejects(d.control.restart('inventory'), { status: 404 });
    await assert.rejects(d.control.restart('../gateway'), { status: 404 });
    await d.control.scale('catalog', 2);
    await assert.rejects(d.control.restart('catalog-3'), { status: 409, message: 'catalog-3 is not running' });
  } finally {
    await d.close();
  }
});

test('scaling stops the highest replicas after draining them and starts the lowest missing ones', async () => {
  const d = await fakeDemo();
  try {
    const down = await d.control.scale('catalog', 1);
    assert.deepEqual(
      { replicas: down.replicas, max: down.max, running: down.running, upstreams: down.upstreams },
      { replicas: 1, max: 3, running: ['catalog-1'], upstreams: [d.url('catalog-1')] },
    );
    assert.deepEqual(d.routed[0], [d.url('catalog-1')], 'routed away before the replicas stop');
    assert.equal(d.supervisor.running('catalog-3'), false);

    const up = await d.control.scale('catalog', 3);
    assert.deepEqual(up.running, ['catalog-1', 'catalog-2', 'catalog-3']);
    assert.deepEqual(up.upstreams, [d.url('catalog-1'), d.url('catalog-2'), d.url('catalog-3')]);
    assert.deepEqual(d.starts.map((s) => s.name), ['catalog-2', 'catalog-3']);

    const same = await d.control.scale('catalog', 3);
    assert.equal(same.replicas, 3);
    assert.equal(d.starts.length, 2, 'scaling to the current size starts nothing');
  } finally {
    await d.close();
  }
});

test('scaling validates the service and the replica count', async () => {
  const d = await fakeDemo();
  try {
    for (const n of [0, 4, 1.5, '2', null, undefined, -1]) {
      await assert.rejects(d.control.scale('catalog', n), { status: 400 }, String(n));
    }
    await assert.rejects(d.control.scale('orders', 2), { status: 404 });
    assert.equal((await d.control.replicas('catalog')).replicas, 3, 'nothing changed');
  } finally {
    await d.close();
  }
});

test('a restarted router gets the current replica list again', async () => {
  const d = await fakeDemo();
  try {
    await d.control.scale('catalog', 2);
    await d.control.restart('gateway');
    assert.deepEqual(d.upstreams(), [d.url('catalog-1'), d.url('catalog-2')]);
  } finally {
    await d.close();
  }
});

test('releases restart the service on another allowlisted version and roll back along the history', async () => {
  const d = await fakeDemo();
  try {
    assert.deepEqual(d.control.release('orders'), { service: 'orders', version: '0.1.0', previous: null, versions: ['0.1.0', '0.2.0'] });
    const deployed = await d.control.deploy('orders', '0.2.0');
    assert.deepEqual([deployed.from_version, deployed.to_version], ['0.1.0', '0.2.0']);
    assert.equal(d.starts.at(-1).env.KARBIRD_SERVICE_VERSION, '0.2.0');
    assert.equal(d.control.instances().find((i) => i.name === 'orders').version, '0.2.0');
    assert.equal(d.control.release('orders').previous, '0.1.0');

    const restarted = await d.control.restart('orders');
    assert.ok(restarted.pid);
    assert.equal(d.starts.at(-1).env.KARBIRD_SERVICE_VERSION, '0.2.0', 'a plain restart keeps the release');

    const back = await d.control.rollback('orders');
    assert.deepEqual([back.from_version, back.to_version], ['0.2.0', '0.1.0']);
    assert.equal(d.starts.at(-1).env.KARBIRD_SERVICE_VERSION, '0.1.0');
    assert.deepEqual(d.control.release('orders'), { service: 'orders', version: '0.1.0', previous: null, versions: ['0.1.0', '0.2.0'] });
    await assert.rejects(d.control.rollback('orders'), { status: 409 }, 'nothing earlier to return to');
  } finally {
    await d.close();
  }
});

test('releases refuse unknown services, unknown versions and no-op deploys', async () => {
  const d = await fakeDemo();
  try {
    for (const v of ['9.9.9', '', undefined, 2, '0.1.0; rm -rf /']) {
      await assert.rejects(d.control.deploy('orders', v), { status: 400 }, String(v));
    }
    await assert.rejects(d.control.deploy('orders', '0.1.0'), { status: 409 });
    await assert.rejects(d.control.deploy('gateway', '0.2.0'), { status: 404 });
    assert.throws(() => d.control.release('gateway'), { status: 404 });
    assert.equal(d.starts.length, 0);
  } finally {
    await d.close();
  }
});

test('a release that never becomes healthy fails and keeps the old version for the next start', async () => {
  const d = await fakeDemo({ broken: new Set(['orders']) });
  try {
    await assert.rejects(d.control.deploy('orders', '0.2.0'), { status: 504 });
    assert.equal(d.control.release('orders').version, '0.1.0');
    assert.equal(d.control.release('orders').previous, null);
  } finally {
    await d.close();
  }
});

test('operations run one at a time and leave routing consistent', async () => {
  const d = await fakeDemo();
  try {
    const results = await Promise.all([d.control.scale('catalog', 1), d.control.restart('catalog-1'), d.control.scale('catalog', 2)]);
    assert.equal(results[2].replicas, 2);
    assert.deepEqual(d.upstreams(), [d.url('catalog-1'), d.url('catalog-2')]);
  } finally {
    await d.close();
  }
});

test('HTTP front: routes, methods, validation and errors as JSON', async () => {
  const d = await fakeDemo();
  const port = await freePort();
  const server = await listenControl(d.control, port);
  const base = `http://127.0.0.1:${port}`;
  const post = (path, body) =>
    fetch(`${base}${path}`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: typeof body === 'string' ? body : JSON.stringify(body ?? {}) });
  try {
    const list = await (await fetch(`${base}/instances`)).json();
    assert.deepEqual(list.instances.map((i) => i.name), NAMES);
    assert.ok(list.instances.every((i) => i.running && Number.isInteger(i.pid)));

    const scaled = await post('/replicas/catalog', { replicas: 2 });
    assert.equal(scaled.status, 200);
    assert.equal((await scaled.json()).replicas, 2);
    assert.equal((await (await fetch(`${base}/replicas/catalog`)).json()).replicas, 2);

    const restarted = await post('/instances/catalog-1/restart');
    assert.equal(restarted.status, 200);
    assert.equal((await restarted.json()).instance, 'catalog-1');

    assert.equal((await post('/releases/orders', { version: '0.2.0' })).status, 200);
    const rolled = await post('/releases/orders/rollback');
    assert.equal(rolled.status, 200);
    assert.equal((await rolled.json()).to_version, '0.1.0');

    const cases = [
      [await post('/replicas/catalog', { replicas: 7 }), 400],
      [await post('/replicas/catalog', '{bad json'), 400],
      [await post('/replicas/catalog', 'x'.repeat(5000)), 413],
      [await post('/instances/catalog-3/restart'), 409],
      [await post('/instances/nope/restart'), 404],
      [await post('/instances/..%2Fgateway/restart'), 404],
      [await post('/releases/orders/rollback'), 409],
      [await fetch(`${base}/instances/catalog-1/restart`), 405],
      [await fetch(`${base}/replicas/catalog`, { method: 'DELETE' }), 405],
      [await fetch(`${base}/`), 404],
      [await fetch(`${base}/shell`), 404],
    ];
    for (const [res, status] of cases) {
      assert.equal(res.status, status, res.url);
      assert.equal(typeof (await res.json()).error, 'string');
    }
  } finally {
    await new Promise((r) => server.close(() => r()));
    await d.close();
  }
});
