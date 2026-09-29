import { test } from 'node:test';
import assert from 'node:assert/strict';
import { cleanEnv, freePort, src, startNode } from './helpers.mjs';

test('payment provider stand-in answers captured (200) or declined (402) with valid JSON', async () => {
  const port = await freePort();
  const p = await startNode(src('provider.mjs'), cleanEnv({ DEMO_PROVIDER_PORT: String(port) }), /listening on/);
  try {
    assert.match(p.output(), new RegExp(`http://127\\.0\\.0\\.1:${port}`));
    const results = await Promise.all(
      Array.from({ length: 60 }, async () => {
        const t0 = performance.now();
        const res = await fetch(`http://127.0.0.1:${port}/v1/charges`, { method: 'POST', body: '{"amount":100}' });
        const text = await res.text();
        return { status: res.status, type: res.headers.get('content-type'), length: Number(res.headers.get('content-length')), text, ms: performance.now() - t0 };
      }),
    );
    for (const r of results) {
      assert.equal(r.type, 'application/json');
      assert.equal(r.length, Buffer.byteLength(r.text));
      const body = JSON.parse(r.text);
      if (r.status === 200) assert.deepEqual(body, { status: 'captured' });
      else {
        assert.equal(r.status, 402);
        assert.deepEqual(body, { status: 'declined' });
      }
      assert.ok(r.ms >= 2.5, `answered in ${r.ms} ms, below the simulated external latency`);
    }
    assert.ok(results.filter((r) => r.status === 200).length > 40, 'declines should be rare');
  } finally {
    await p.stop();
  }
});
