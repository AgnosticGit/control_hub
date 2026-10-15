import { test } from 'node:test';
import assert from 'node:assert/strict';
import { INCIDENT_BURST_MS, INCIDENT_CYCLE_MS, demoIncidentErrorRate, startDemoIncidentCycle } from '../demo-incidents.mjs';

test('periodic demo incident bursts last three minutes in each ten-minute cycle', () => {
  assert.equal(demoIncidentErrorRate(0), 0.5);
  assert.equal(demoIncidentErrorRate(INCIDENT_BURST_MS - 1), 0.5);
  assert.equal(demoIncidentErrorRate(INCIDENT_BURST_MS), 0);
  assert.equal(demoIncidentErrorRate(INCIDENT_CYCLE_MS - 1), 0);
  assert.equal(demoIncidentErrorRate(INCIDENT_CYCLE_MS), 0.5);
  assert.equal(demoIncidentErrorRate(-1), 0);
});

test('the cycle instructs load generator to replace normal requests during a burst', async () => {
  const requests = [];
  const cycle = startDemoIncidentCycle({
    loadControlUrl: 'http://127.0.0.1:7091/',
    intervalMs: 60_000,
    fetchImpl: async (url, options) => {
      requests.push({ url, options });
      return { ok: true, status: 204 };
    },
  });
  await new Promise((resolve) => setImmediate(resolve));
  cycle.setEnabled(false);
  await new Promise((resolve) => setImmediate(resolve));
  cycle.stop();
  assert.equal(requests.length, 2);
  assert.equal(requests[0].url, 'http://127.0.0.1:7091/__demo/incident-state');
  assert.equal(requests[0].options.method, 'POST');
  assert.deepEqual(JSON.parse(requests[0].options.body), { active: true });
  assert.deepEqual(JSON.parse(requests[1].options.body), { active: false });
});
