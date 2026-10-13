// Real OTLP -> Kafka -> ClickHouse -> incident evaluator -> scoped API.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { API, DEMO, ISO, ORIGIN, Session, encodeTraces, eventually, hex, nowNs, otlpHttp } from './lib.mjs';

test('incident rule opens on two closed breaching windows and remains tenant scoped', async () => {
  const owner = new Session();
  await owner.login('isolation@karbird.local');
  const viewer = new Session();
  await viewer.login('viewer@karbird.local');
  const service = `incident-it-${randomUUID().slice(0, 8)}`;
  const rule = { service_id: service, kind: 'error_rate', threshold: 10, min_requests: 20 };
  const path = `${ISO}/incident-rules`;

  assert.equal((await viewer.json(path)).status, 404, 'foreign tenant cannot read rules');
  assert.equal((await viewer.mutate(`${DEMO}/incident-rules`, rule)).status, 403, 'viewer cannot configure rules');
  assert.equal((await owner.json(path, {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify(rule),
  })).status, 403, 'cookie mutation needs CSRF');
  assert.equal((await fetch(`${API}${path}`)).status, 401, 'anonymous read is rejected');

  // The same 24 failed server spans lie in both adjacent five-minute windows.
  // They are ingested now, so the processor watermark also proves completeness.
  const minute = 60_000_000_000n;
  const end = ((nowNs() - 2n * minute) / minute) * minute;
  const at = end - 3n * minute;
  const spans = Array.from({ length: 24 }, () => ({
    traceId: hex(16), spanId: hex(8), name: 'GET /incident-test', kind: 2,
    startNs: at, endNs: at + 20_000_000n, status: { code: 2, message: 'error' },
  }));
  assert.equal((await otlpHttp('traces', encodeTraces(service, spans))).status, 200);
  await eventually(async () => {
    const summary = await owner.json(`${ISO}/services?from=${end - 5n * minute}&to=${end}`);
    assert.equal(summary.status, 200);
    const row = summary.body.services.find((item) => item.name === service);
    assert.ok(row?.requests >= 20, 'waiting for ClickHouse readback');
    assert.ok(row?.errors >= 20);
    rule.service_id = row.service_id;
  }, { timeoutMs: 30_000, intervalMs: 1000 });

  const created = await owner.mutate(path, rule);
  assert.equal(created.status, 200, JSON.stringify(created.body));
  assert.equal(created.body.observation_state, 'unknown');
  assert.equal((await owner.mutate(path, rule)).status, 409, 'one rule per service and metric');

  try {
    const opened = await eventually(async () => {
      const result = await owner.json(`${ISO}/incidents`);
      assert.equal(result.status, 200);
      const incident = result.body.find((item) => item.rule_id === created.body.id);
      assert.ok(incident, 'waiting for two processed windows');
      assert.equal(incident.state, 'open');
      assert.ok(incident.requests >= 20);
      assert.ok(incident.errors >= 20);
      return incident;
    }, { timeoutMs: 90_000, intervalMs: 2000 });

    assert.equal((await viewer.json(`${DEMO}/incidents/${opened.id}`)).status, 404);
    const acknowledged = await owner.mutate(`${ISO}/incidents/${opened.id}/acknowledge`, {});
    assert.equal(acknowledged.status, 200);
    assert.equal(acknowledged.body.acknowledged, true);
    assert.equal((await owner.json(`${ISO}/incidents/${opened.id}`)).body.acknowledged, true);
  } finally {
    const paused = await owner.mutate(`${path}/${created.body.id}`, {
      threshold: rule.threshold, min_requests: rule.min_requests, enabled: false,
    }, 'PATCH');
    assert.equal(paused.status, 200, JSON.stringify(paused.body));
  }
});
