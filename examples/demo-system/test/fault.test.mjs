import { test } from 'node:test';
import assert from 'node:assert/strict';
import { captureServer, cleanEnv, runNode, src } from './helpers.mjs';

const fault = src('fault.mjs');

async function withTarget(fn, handler) {
  const cap = await captureServer(handler);
  try {
    return await fn(cap, cleanEnv({ DEMO_ORDERS_URL: cap.url }));
  } finally {
    await cap.close();
  }
}

test('prints usage and exits 2 for a missing or unknown service', async () => {
  for (const args of [[], ['billing']]) {
    const r = await runNode(fault, args, cleanEnv());
    assert.equal(r.code, 2);
    assert.match(r.stderr, /usage: fault\.mjs <gateway\|orders\|inventory\|payments\|notifications\|catalog\|auth>/);
  }
});

test('rejects unknown arguments', async () => {
  const r = await runNode(fault, ['orders', '--latency', '5', '--rm-rf'], cleanEnv());
  assert.equal(r.code, 2);
  assert.match(r.stderr, /unknown argument: --rm-rf/);
});

test('rejects missing or non-numeric values instead of silently clearing the fault', async () => {
  await withTarget(async (cap, env) => {
    for (const args of [['--latency'], ['--latency', 'abc'], ['--error-rate', ''], ['--error-rate', 'NaN']]) {
      const r = await runNode(fault, ['orders', ...args], env);
      assert.equal(r.code, 2, args.join(' '));
      assert.match(r.stderr, /needs a number/);
    }
    assert.equal(cap.requests.length, 0);
  });
});

test('posts latency and error rate to the fault endpoint of the chosen service', async () => {
  await withTarget(async (cap, env) => {
    const r = await runNode(fault, ['orders', '--latency', '800', '--error-rate', '0.3'], env);
    assert.equal(r.code, 0, r.stderr);
    assert.equal(cap.requests.length, 1);
    const [req] = cap.requests;
    assert.equal(req.method, 'POST');
    assert.equal(req.url, '/__demo/faults');
    assert.equal(req.headers['content-type'], 'application/json');
    assert.deepEqual(JSON.parse(req.body), { latencyMs: 800, errorRate: 0.3 });
    assert.match(r.stdout, /^orders: \{"ok":true\}/);
  });
});

test('--clear resets both values, a single option defaults the other to 0', async () => {
  await withTarget(async (cap, env) => {
    await runNode(fault, ['orders', '--clear'], env);
    await runNode(fault, ['orders', '--error-rate', '1'], env);
    assert.deepEqual(
      cap.requests.map((r) => JSON.parse(r.body)),
      [
        { latencyMs: 0, errorRate: 0 },
        { latencyMs: 0, errorRate: 1 },
      ],
    );
  });
});

test('--status only reads', async () => {
  await withTarget(async (cap, env) => {
    const r = await runNode(fault, ['orders', '--status'], env);
    assert.equal(r.code, 0);
    assert.equal(cap.requests[0].method, 'GET');
    assert.equal(cap.requests[0].body, '');
  });
});

test('exits 1 with a hint when the service refuses (faults disabled)', async () => {
  await withTarget(
    async (_cap, env) => {
      const r = await runNode(fault, ['orders', '--latency', '10'], env);
      assert.equal(r.code, 1);
      assert.match(r.stderr, /fault control failed \(404\)/);
      assert.match(r.stderr, /CONTROLHUB_DEMO_FAULTS=1/);
    },
    (_req, res) => {
      res.writeHead(404, { 'content-type': 'application/json' });
      res.end('{"error":"not found"}');
    },
  );
});
