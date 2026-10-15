// Real OTLP metrics through ingest, Kafka, processor and ClickHouse to the scoped overview API.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { API, DEMO, ISO, Session, encodeMetrics, eventually, nowNs, otlpHttp } from './lib.mjs';

test('histograms and dependency statistics appear only in the authenticated environment', async () => {
  const iso = new Session();
  const admin = new Session();
  await iso.login('isolation@karbird.local');
  await admin.login('admin@karbird.local');
  const suffix = randomUUID().slice(0, 8);
  const service = `it-metrics-${suffix}`;
  const database = `it-postgresql-${suffix}`;
  const at = nowNs();
  const first = at - 20_000_000_000n;
  const last = at - 5_000_000_000n;
  const histogram = (timeNs, count, bucketCounts) => ({
    timeNs, startNs: first - 1_000_000_000n, count, value: count * 0.2,
    bounds: [0.1, 0.5], bucketCounts,
    attributes: { 'http.route': '/orders/{id}' },
  });
  const body = encodeMetrics([
    { attributes: { 'service.name': service }, metrics: [
      { name: 'http.server.request.duration', unit: 's', kind: 'histogram',
        points: [histogram(first, 10, [2, 8, 0]), histogram(last, 20, [4, 16, 0])] },
      { name: 'process.memory.usage', unit: 'By', kind: 'gauge',
        points: [{ timeNs: last, value: 2_097_152, attributes: { type: 'rss' } }] },
    ] },
    { attributes: { 'service.name': database, 'db.system.name': 'postgresql' }, metrics: [
      { name: 'postgresql.backends', kind: 'gauge', points: [{ timeNs: last, value: 3 }] },
    ] },
  ]);
  const ingest = await otlpHttp('metrics', body);
  assert.equal(ingest.status, 200, ingest.body.toString());
  const query = (name) => `${ISO}/metrics/overview?${new URLSearchParams({ name, at: at.toString() })}`;
  const result = await eventually(async () => {
    const response = await iso.json(query(service));
    assert.equal(response.status, 200, JSON.stringify(response.body));
    assert.equal(response.body.http_samples, 10);
    return response.body;
  });
  assert.equal(result.http_p95_ms, 500);
  assert.equal(result.process_rss_bytes, 2_097_152);
  const dependency = await iso.json(query(database));
  assert.equal(dependency.status, 200);
  assert.equal(dependency.body.postgres_backends, 3);
  assert.equal((await fetch(`${API}${query(service)}`)).status, 401);
  assert.equal((await admin.json(query(service))).status, 404);
  const other = await admin.json(`${DEMO}/metrics/overview?name=${encodeURIComponent(service)}&at=${at}`);
  assert.equal(other.status, 200);
  assert.equal(other.body.http_samples, 0, 'same service name in another environment has no data');
});
