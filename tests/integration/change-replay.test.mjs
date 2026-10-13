import { test } from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { API, OTLP, ISO, DEMO, creds, Session, eventually, nowNs, hex, encodeTraces, otlpHttp } from './lib.mjs';

test('deployment events can be filtered and their before/after service windows remain scoped', async () => {
  const iso = new Session();
  await iso.login('isolation@karbird.local');
  const at = nowNs() - 3n * 60n * 1_000_000_000n;
  const service = `release-${hex(5)}`;
  const releaseId = randomUUID();
  const unrelatedId = randomUUID();
  const before = at - 30n * 1_000_000_000n;
  const after = at + 30n * 1_000_000_000n;

  const spans = encodeTraces(service, [
    { traceId: hex(16), spanId: hex(8), name: 'before', kind: 2, startNs: before, endNs: before + 1_000_000n },
    { traceId: hex(16), spanId: hex(8), name: 'after', kind: 2, startNs: after, endNs: after + 3_000_000n, status: { code: 2 } },
  ]);
  assert.equal((await otlpHttp('traces', spans)).status, 200);
  const sent = await fetch(`${OTLP}/api/v1/events/batch`, {
    method: 'POST',
    headers: { authorization: `Bearer ${creds.ingest_tokens['isolation-test']}`, 'content-type': 'application/json' },
    body: JSON.stringify({ events: [
      { event_id: releaseId, event_type: 'deployment.finished', service: { name: service }, event_time: at.toString(), attributes: { 'service.version': '2.0.0' } },
      { event_id: unrelatedId, event_type: 'deployment.started', service: { name: service }, event_time: at.toString() },
    ] }),
  });
  assert.equal(sent.status, 200);
  assert.equal((await sent.json()).accepted, 2);

  const eventPath = `${ISO}/events?from=${at - 1n}&to=${at + 1n}&event_type=deployment.finished`;
  await eventually(async () => {
    const result = await iso.json(eventPath);
    assert.equal(result.status, 200);
    assert.deepEqual(result.body.events.map((event) => event.id), [releaseId]);
    assert.equal(result.body.events[0].attributes['service.version'], '2.0.0');
    const prior = await iso.json(`${ISO}/services?from=${at - 60n * 1_000_000_000n}&to=${at}`);
    const later = await iso.json(`${ISO}/services?from=${at}&to=${at + 60n * 1_000_000_000n}`);
    const a = prior.body.services.find((row) => row.name === service);
    const b = later.body.services.find((row) => row.name === service);
    assert.equal(a?.requests, 1);
    assert.equal(a?.errors, 0);
    assert.equal(b?.requests, 1);
    assert.equal(b?.errors, 1);
  });

  assert.equal((await fetch(`${API}${eventPath}`)).status, 401);
  assert.equal((await iso.json(`${DEMO}/events?from=${at - 1n}&to=${at + 1n}&event_type=deployment.finished`)).status, 404);
  const demo = new Session();
  await demo.login('admin@karbird.local');
  assert.equal((await demo.json(eventPath)).status, 404);
});
