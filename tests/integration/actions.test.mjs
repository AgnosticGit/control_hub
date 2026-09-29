// Controlled actions end to end: propose → approve (four-eyes) → executor → verification → audit.
// Needs `just dev` (with the executor) and `just demo` running.
// Run: node --test "tests/integration/actions.test.mjs"
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { Session, DEMO, scoped, bearer, eventually } from './lib.mjs';

const ISO_PROD = scoped('isolation', 'isolation-prod');
const OPEN = new Set(['proposed', 'approved', 'running']);
const FINAL = new Set(['succeeded', 'failed', 'verification_failed', 'rejected', 'cancelled', 'expired']);

let admin;
let operator;
let viewer;

before(async () => {
  admin = new Session();
  await admin.login('admin@controlhub.local');
  operator = new Session();
  await operator.login('operator@controlhub.local');
  viewer = new Session();
  await viewer.login('viewer@controlhub.local');
  // Leftovers from an interrupted run would trip the one-open-plan-per-target policy.
  const list = await admin.json(`${DEMO}/actions`);
  for (const p of list.body.filter((p) => p.state === 'proposed' || p.state === 'approved')) {
    await admin.mutate(`${DEMO}/actions/${p.id}/cancel`, { comment: 'integration test cleanup' });
  }
  await eventually(async () => {
    const l = await admin.json(`${DEMO}/actions`);
    assert.ok(!l.body.some((p) => OPEN.has(p.state)), 'waiting for running plans to finish');
  }, { timeoutMs: 90_000, intervalMs: 2000 });
});

const propose = (session, action_type, target, params = {}, base = DEMO) =>
  session.mutate(`${base}/actions`, { action_type, target, params, reason: `integration test ${randomUUID().slice(0, 8)}` });

async function waitFinal(id) {
  return eventually(async () => {
    const d = await admin.json(`${DEMO}/actions/${id}`);
    assert.equal(d.status, 200);
    assert.ok(FINAL.has(d.body.plan.state), `plan still ${d.body.plan.state}`);
    return d.body;
  }, { timeoutMs: 120_000, intervalMs: 2000 });
}

test('catalog is visible to viewers and reflects the demo-only policy', async () => {
  const demo = await viewer.json(`${DEMO}/actions/catalog`);
  assert.equal(demo.status, 200);
  assert.equal(demo.body.environment_is_demo, true);
  assert.deepEqual(demo.body.actions.map((a) => a.action_type).sort(), ['demo.faults.clear', 'demo.faults.inject', 'demo.restart_database']);
  assert.ok(demo.body.actions.every((a) => a.allowed_here));

  const iso = new Session();
  await iso.login('isolation@controlhub.local');
  const prod = await iso.json(`${ISO_PROD}/actions/catalog`);
  assert.equal(prod.status, 200);
  assert.equal(prod.body.environment_is_demo, false);
  assert.ok(prod.body.actions.every((a) => !a.allowed_here));
  assert.equal((await iso.json(`${DEMO}/actions/catalog`)).status, 404, 'other tenant');
});

test('who may propose: not viewers, not API tokens, not in non-demo environments', async () => {
  assert.equal((await propose(viewer, 'demo.faults.clear', 'orders')).status, 403);

  const tok = await admin.mutate(`${DEMO}/tokens`, { kind: 'api', name: `actions-${randomUUID().slice(0, 8)}` });
  assert.equal(tok.status, 200);
  const viaToken = await bearer(tok.body.secret, `${DEMO}/actions`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ action_type: 'demo.faults.clear', target: 'orders', params: {}, reason: 'token attempt' }),
  });
  assert.equal(viaToken.status, 403, 'API tokens are read-only');
  assert.equal((await bearer(tok.body.secret, `${DEMO}/actions`)).status, 200, 'but may read plans');
  await admin.mutate(`${DEMO}/tokens/${tok.body.token.id}/revoke`, {});

  const iso = new Session();
  await iso.login('isolation@controlhub.local');
  const denied = await propose(iso, 'demo.faults.clear', 'orders', {}, ISO_PROD);
  assert.equal(denied.status, 403);
  assert.equal(denied.body.code, 'policy_denied');
  assert.match(denied.body.message, /demo environment/);
  const audit = await iso.json(`${ISO_PROD}/audit`);
  assert.ok(audit.body.some((a) => a.action === 'action.propose' && a.outcome === 'denied'), 'denials are audited');

  assert.equal((await propose(operator, 'demo.shell', 'orders')).status, 400, 'not in the catalog');
  assert.equal((await propose(operator, 'demo.faults.inject', 'orders', { latency_ms: 10, error_rate: 2 })).status, 400);
  assert.equal((await propose(operator, 'demo.faults.inject', 'postgres', { latency_ms: 10, error_rate: 0.1 })).status, 400);
  assert.equal((await propose(operator, 'demo.faults.clear', 'orders', { cmd: 'rm -rf /' })).status, 400, 'unknown params');
});

test('four-eyes: proposer cannot approve; reject needs a comment; only proposer or admin can cancel', async () => {
  const p = await propose(admin, 'demo.faults.clear', 'gateway');
  assert.equal(p.status, 200, JSON.stringify(p.body));
  assert.equal(p.body.state, 'proposed');
  const id = p.body.id;

  assert.equal((await admin.mutate(`${DEMO}/actions/${id}/approve`, {})).status, 403, 'self-approval');
  assert.equal((await operator.mutate(`${DEMO}/actions/${id}/approve`, {})).status, 403, 'operators cannot approve');
  assert.equal((await operator.mutate(`${DEMO}/actions/${id}/cancel`, {})).status, 403, "cannot cancel someone else's plan");

  const second = await propose(operator, 'demo.faults.clear', 'gateway');
  assert.equal(second.status, 403);
  assert.match(second.body.message, /open plan/);

  assert.equal((await admin.mutate(`${DEMO}/actions/${id}/reject`, {})).status, 400);
  const rejected = await admin.mutate(`${DEMO}/actions/${id}/reject`, { comment: 'not needed' });
  assert.equal(rejected.status, 200);
  assert.equal(rejected.body.state, 'rejected');
  assert.equal((await admin.mutate(`${DEMO}/actions/${id}/cancel`, {})).status, 409, 'final states are final');

  const c = await propose(operator, 'demo.faults.clear', 'gateway');
  assert.equal(c.status, 200);
  const cancelled = await operator.mutate(`${DEMO}/actions/${c.body.id}/cancel`, { comment: 'changed my mind' });
  assert.equal(cancelled.status, 200);
  assert.equal(cancelled.body.state, 'cancelled');
});

test('approved fault injection is executed, verified and audited; clearing restores the service', async () => {
  const p = await propose(operator, 'demo.faults.inject', 'orders', { latency_ms: 20, error_rate: 0.3 });
  assert.equal(p.status, 200, JSON.stringify(p.body));
  assert.ok(p.body.policy.allowed);
  const approved = await admin.mutate(`${DEMO}/actions/${p.body.id}/approve`, { comment: 'go' });
  assert.equal(approved.status, 200, JSON.stringify(approved.body));
  assert.equal(approved.body.state, 'approved');
  assert.equal(approved.body.decided_by, 'admin@controlhub.local');

  const done = await waitFinal(p.body.id);
  assert.equal(done.plan.state, 'succeeded', JSON.stringify(done.plan));
  assert.ok(done.plan.executor_id);
  assert.equal(done.plan.result.fault_state.errorRate, 0.3);
  assert.equal(done.plan.verification.passed, true);
  assert.ok(done.plan.verification.requests > 0);
  assert.deepEqual(
    done.audit.map((a) => a.action).sort(),
    ['action.approve', 'action.finish', 'action.propose', 'action.start'],
  );
  assert.ok(done.audit.find((a) => a.action === 'action.start').actor_kind !== 'user');

  const clear = await propose(operator, 'demo.faults.clear', 'orders');
  assert.equal(clear.status, 200, JSON.stringify(clear.body));
  assert.equal((await admin.mutate(`${DEMO}/actions/${clear.body.id}/approve`, {})).status, 200);
  const cleared = await waitFinal(clear.body.id);
  assert.equal(cleared.plan.state, 'succeeded', JSON.stringify(cleared.plan));
  assert.equal(cleared.plan.result.fault_state.errorRate, 0);
  assert.ok(cleared.plan.verification.errors / cleared.plan.verification.requests <= 0.05);
});

test('database restart goes through the executor and the service recovers', { skip: process.env.CONTROLHUB_TEST_RESTART === '0' }, async () => {
  const p = await propose(operator, 'demo.restart_database', 'demo-postgres');
  assert.equal(p.status, 200, JSON.stringify(p.body));
  assert.equal((await admin.mutate(`${DEMO}/actions/${p.body.id}/approve`, {})).status, 200);
  const done = await waitFinal(p.body.id);
  assert.equal(done.plan.state, 'succeeded', JSON.stringify(done.plan));
  assert.equal(done.plan.result.container, 'controlhub-demo-postgres-1');
});
