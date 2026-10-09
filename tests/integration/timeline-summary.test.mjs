// Month-scale summaries are bounded, scoped and keep nanosecond boundaries as strings.
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { API, DEMO, ISO, Session, nowNs } from './lib.mjs';

const DAY_NS = 86_400_000_000_000n;
const to = nowNs();
const from = to - 30n * DAY_NS;
const query = `from=${from}&to=${to}&buckets=1000`;

let admin;
let isolation;
before(async () => {
  admin = new Session();
  await admin.login('admin@karbird.local');
  isolation = new Session();
  await isolation.login('isolation@karbird.local');
});

test('timeline month summary is bounded and returns decimal nanosecond ranges', async () => {
  const r = await admin.json(`${DEMO}/timeline/summary?${query}`);
  assert.equal(r.status, 200, JSON.stringify(r.body));
  assert.deepEqual([r.body.from_ns, r.body.to_ns], [from.toString(), to.toString()]);
  assert.equal(r.body.buckets, 1000);
  assert.ok(BigInt(r.body.bucket_ns) > 0n);
  assert.equal(r.body.signals.length, 1000);
  for (const service of r.body.services) assert.equal(service.spans.length, 1000);
  for (const host of r.body.hosts) {
    assert.equal(host.cpu.length, 1000);
    assert.equal(host.memory.length, 1000);
  }
});

test('timeline month summary rejects wider ranges and enforces project scope', async () => {
  assert.equal((await admin.json(`${DEMO}/timeline/summary?from=${from - 1n}&to=${to}`)).status, 400);
  assert.equal((await admin.json(`${ISO}/timeline/summary?${query}`)).status, 404);
  assert.equal((await isolation.json(`${DEMO}/timeline/summary?${query}`)).status, 404);
  assert.equal((await fetch(`${API}${DEMO}/timeline/summary?${query}`)).status, 401);
});
