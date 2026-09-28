import { test } from 'node:test';
import assert from 'node:assert/strict';
import { EventsClient, nowUnixNanos } from '../src/events.js';

function client(fetchImpl) {
  return new EventsClient({
    url: 'http://127.0.0.1:1/api/v1/events/batch',
    headers: { authorization: 'Bearer t' },
    serviceName: 'svc',
    serviceNamespace: '',
    instanceId: 'i1',
    fetchImpl,
  });
}

test('nowUnixNanos is a decimal nanosecond string close to Date.now()', () => {
  const ns = BigInt(nowUnixNanos());
  const diffMs = Number(ns / 1_000_000n) - Date.now();
  assert.ok(Math.abs(diffMs) < 1000);
});

test('sends batch with sequence numbers and drops invalid trace ids', async () => {
  const bodies = [];
  const c = client(async (_url, init) => {
    bodies.push(JSON.parse(init.body));
    return new Response(JSON.stringify({ accepted: 2, rejected: [] }), { status: 202 });
  });
  c.push('request.started', { traceId: 'ab'.repeat(16), spanId: 'cd'.repeat(8) });
  c.push('request.completed', { traceId: 'not-hex' });
  await c.flush();
  assert.equal(bodies.length, 1);
  const [a, b] = bodies[0].events;
  assert.equal(a.source.sequence, 0);
  assert.equal(b.source.sequence, 1);
  assert.equal(a.trace_id, 'ab'.repeat(16));
  assert.equal(b.trace_id, undefined);
  assert.deepEqual(c.stats(), { queued: 2, sent: 2, rejected: 0, dropped: 0, retried: 0, failed: 0, pending: 0 });
});

test('retries 503 and does not retry 400', async () => {
  let calls = 0;
  const c = client(async () => {
    calls++;
    return new Response('{}', { status: calls === 1 ? 503 : 400 });
  });
  c.push('request.started');
  await c.flush();
  assert.equal(calls, 2);
  assert.equal(c.stats().retried, 1);
  assert.equal(c.stats().rejected, 1);
});
