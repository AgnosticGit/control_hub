// Broker outage: with Kafka stopped, ingestion must answer 503 (never 200) and count the failure;
// after Kafka is back the flow recovers without manual steps.
//
// Disruptive (stops the shared Kafka container), so it only runs on request:
//   just test-broker-outage        (= node tests/integration/broker-outage.test.mjs --run)
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { join } from 'node:path';
import { DEMO, OTLP, Session, eventually, root } from './lib.mjs';

const enabled = process.argv.includes('--run') || process.env.KARBIRD_TEST_BROKER_OUTAGE === '1';
const METRICS = 'http://127.0.0.1:9464/metrics';

function compose(...args) {
  const r = spawnSync(
    'docker',
    ['compose', '--env-file', join(root, '.karbird', 'infra.env'), '-f', join(root, 'infra', 'compose', 'compose.yaml'), ...args],
    { cwd: root, stdio: 'inherit' },
  );
  assert.equal(r.status, 0, `docker compose ${args.join(' ')}`);
}

async function failedBroker() {
  const text = await (await fetch(METRICS)).text();
  const m = text.match(/karbird_ingest_requests_total\{result="failed_broker"\} (\d+)/);
  return Number(m?.[1] ?? 0);
}

test('ingestion refuses (503) during a broker outage and recovers afterwards', { skip: !enabled && 'disruptive; run with `just test-broker-outage`', timeout: 300_000 }, async () => {
  const admin = new Session();
  await admin.login('admin@karbird.local');
  const tok = await admin.mutate(`${DEMO}/tokens`, { kind: 'ingest', name: `outage-${randomUUID().slice(0, 8)}` });
  assert.equal(tok.status, 200);
  const send = async (id) => {
    const res = await fetch(`${OTLP}/api/v1/events/batch`, {
      method: 'POST',
      headers: { authorization: `Bearer ${tok.body.secret}`, 'content-type': 'application/json' },
      body: JSON.stringify({
        events: [{ event_id: id, event_type: 'outage.test', service: { name: 'outage-test' }, event_time: `${BigInt(Date.now()) * 1_000_000n}` }],
      }),
    });
    return { status: res.status, body: await res.json().catch(() => null) };
  };

  assert.equal((await send(randomUUID())).status, 200, 'baseline');
  const before = await failedBroker();

  compose('stop', 'kafka');
  try {
    const started = Date.now();
    const during = await send(randomUUID());
    const ms = Date.now() - started;
    assert.equal(during.status, 503, `expected 503 without a broker, got ${during.status} ${JSON.stringify(during.body)}`);
    console.log(`503 after ${ms} ms: ${JSON.stringify(during.body)}`);
    assert.ok((await failedBroker()) > before, 'failed_broker counter increased');
  } finally {
    compose('up', '-d', '--wait', '--wait-timeout', '180', 'kafka');
  }

  const id = randomUUID();
  await eventually(async () => assert.equal((await send(id)).status, 200), { timeoutMs: 120_000, intervalMs: 3000 });
  await eventually(async () => {
    const ev = await admin.json(`${DEMO}/events?limit=200`);
    assert.ok(ev.body.events.some((e) => e.id === id), 'event after recovery is queryable');
  }, { timeoutMs: 60_000, intervalMs: 2000 });
});
