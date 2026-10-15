// Controlled actions end to end: propose → approve (or run at once by someone allowed to approve)
// → executor → verification → audit.
// Needs `just dev` (with the executor) and `just demo` running.
// Run: node --test "tests/integration/actions.test.mjs"
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { Session, DEMO, ORIGIN, scoped, bearer, eventually } from './lib.mjs';

const ISO_PROD = scoped('isolation', 'isolation-prod');
const OPEN = new Set(['proposed', 'approved', 'running']);
const FINAL = new Set(['succeeded', 'failed', 'verification_failed', 'rejected', 'cancelled', 'expired']);
// Demo endpoints (loopback): used only to put the demo back into its default state.
const DEMO_CONTROL = process.env.DEMO_CONTROL_URL ?? 'http://127.0.0.1:7090';
const DEMO_GATEWAY = process.env.DEMO_GATEWAY_URL ?? 'http://127.0.0.1:7001';
const DEMO_ORDERS = process.env.DEMO_ORDERS_URL ?? 'http://127.0.0.1:7002';

let admin;
let operator;
let viewer;

const postJson = (url, body) =>
  fetch(url, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });

/** Flags off, three catalog replicas, orders on 0.1.0 — whatever an interrupted run left behind. */
async function resetDemo() {
  await postJson(`${DEMO_ORDERS}/__demo/flags`, { flag: 'checkout-v2', enabled: false });
  await postJson(`${DEMO_GATEWAY}/__demo/flags`, { flag: 'catalog-cache', enabled: false });
  const replicas = await (await fetch(`${DEMO_CONTROL}/replicas/catalog`)).json();
  if (replicas.replicas !== 3) assert.equal((await postJson(`${DEMO_CONTROL}/replicas/catalog`, { replicas: 3 })).status, 200);
  const release = await (await fetch(`${DEMO_CONTROL}/releases/orders`)).json();
  if (release.version !== '0.1.0') assert.equal((await postJson(`${DEMO_CONTROL}/releases/orders`, { version: '0.1.0' })).status, 200);
}

after(async () => {
  await resetDemo();
  assert.equal((await postJson(`${DEMO_CONTROL}/incident-cycle`, { enabled: true })).status, 200);
});

before(async () => {
  assert.equal((await postJson(`${DEMO_CONTROL}/incident-cycle`, { enabled: false })).status, 200);
  await resetDemo();
  admin = new Session();
  await admin.login('admin@karbird.local');
  operator = new Session();
  await operator.login('operator@karbird.local');
  viewer = new Session();
  await viewer.login('viewer@karbird.local');
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
  assert.deepEqual(demo.body.actions.map((a) => a.action_type).sort(), [
    'demo.container.restart',
    'demo.faults.clear',
    'demo.faults.inject',
    'demo.feature_flag.set',
    'demo.release.rollback',
    'demo.replicas.scale',
    'demo.restart_database',
  ]);
  assert.ok(demo.body.actions.every((a) => a.allowed_here));
  const byType = Object.fromEntries(demo.body.actions.map((a) => [a.action_type, a]));
  assert.deepEqual(byType['demo.container.restart'].targets, ['gateway', 'orders', 'catalog-1', 'catalog-2', 'catalog-3']);
  assert.deepEqual(byType['demo.feature_flag.set'].params.map((p) => [p.name, p.boolean, p.min, p.max]), [['enabled', true, 0, 1]]);
  assert.deepEqual(byType['demo.replicas.scale'].params.map((p) => [p.name, p.boolean, p.min, p.max]), [['replicas', false, 1, 3]]);
  // Every target says which map card (and container) it belongs to, for the System Map controls.
  for (const a of demo.body.actions) assert.deepEqual(a.subjects.map((s) => s.target), a.targets, a.action_type);
  assert.deepEqual(
    byType['demo.container.restart'].subjects.find((s) => s.target === 'catalog-2'),
    { target: 'catalog-2', service: 'catalog', container: 'catalog-2' },
  );
  assert.deepEqual(byType['demo.restart_database'].subjects, [{ target: 'demo-postgres', service: 'postgresql: orders', container: 'postgres' }]);
  assert.deepEqual(byType['demo.feature_flag.set'].subjects.map((s) => [s.target, s.service]), [['catalog-cache', 'gateway'], ['checkout-v2', 'orders']]);
  // Only roles with the approve permission run actions without a second approval.
  assert.equal(demo.body.can_run, false, 'viewer');
  assert.equal((await operator.json(`${DEMO}/actions/catalog`)).body.can_run, false, 'operator');
  assert.equal((await admin.json(`${DEMO}/actions/catalog`)).body.can_run, true, 'admin');

  const iso = new Session();
  await iso.login('isolation@karbird.local');
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
  await iso.login('isolation@karbird.local');
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

test('new demo actions refuse foreign targets and out-of-range parameters before any plan exists', async () => {
  const before = (await admin.json(`${DEMO}/actions`)).body.length;
  for (const [type, target, params] of [
    ['demo.container.restart', 'demo-postgres', {}],
    ['demo.container.restart', 'catalog-1; docker ps', {}],
    ['demo.replicas.scale', 'catalog', { replicas: 9 }],
    ['demo.replicas.scale', 'catalog', { replicas: 0 }],
    ['demo.replicas.scale', 'orders', { replicas: 2 }],
    ['demo.feature_flag.set', 'checkout-v2', { enabled: 0.5 }],
    ['demo.feature_flag.set', 'kill-switch', { enabled: 1 }],
    ['demo.release.rollback', 'orders', { version: '0.0.1' }],
    ['demo.release.rollback', 'gateway', {}],
  ]) {
    const r = await propose(operator, type, target, params);
    assert.equal(r.status, 400, `${type} ${target} ${JSON.stringify(params)}: ${JSON.stringify(r.body)}`);
  }
  assert.equal((await propose(viewer, 'demo.container.restart', 'catalog-1')).status, 403);
  // Running at once still needs the CSRF header like every cookie-authenticated mutation.
  const noCsrf = await admin.json(`${DEMO}/actions`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify({ action_type: 'demo.container.restart', target: 'catalog-1', params: {}, reason: 'no csrf token' }),
  });
  assert.equal(noCsrf.status, 403, JSON.stringify(noCsrf.body));
  assert.equal((await admin.json(`${DEMO}/actions`)).body.length, before, 'no plan was created');
});

/** Operator proposes, admin approves, the executor runs and verifies. */
async function approvedRun(type, target, params = {}) {
  const p = await propose(operator, type, target, params);
  assert.equal(p.status, 200, JSON.stringify(p.body));
  const approved = await admin.mutate(`${DEMO}/actions/${p.body.id}/approve`, {});
  assert.equal(approved.status, 200, JSON.stringify(approved.body));
  return waitFinal(p.body.id);
}

test('an admin runs a container restart at once: it replaces the process and the restarted instance serves traffic', async () => {
  const p = await propose(admin, 'demo.container.restart', 'catalog-2');
  assert.equal(p.status, 200, JSON.stringify(p.body));
  assert.equal(p.body.state, 'approved', 'no second person needed');
  assert.equal(p.body.decided_by, 'admin@karbird.local');
  assert.equal(p.body.created_by, 'admin@karbird.local');
  assert.equal(p.body.policy.requires_approval, false);
  const done = await waitFinal(p.body.id);
  assert.equal(done.plan.state, 'succeeded', JSON.stringify(done.plan));
  assert.deepEqual(
    done.audit.map((a) => a.action).sort(),
    ['action.approve', 'action.finish', 'action.propose', 'action.start'],
    'the approval is audited even when the proposer approved',
  );
  const { result, verification } = done.plan;
  assert.equal(result.container, 'catalog-2');
  assert.ok(Number.isInteger(result.pid) && result.pid !== result.previous_pid, JSON.stringify(result));
  assert.equal(verification.service, 'catalog');
  assert.equal(verification.instance, 'catalog-2');
  assert.ok(verification.requests > 0 && verification.passed, JSON.stringify(verification));
  const instances = await (await fetch(`${DEMO_CONTROL}/instances`)).json();
  assert.equal(instances.instances.find((i) => i.name === 'catalog-2').pid, result.pid, 'the demo runs the new process');
});

test('scaling catalog down and back up is verified by the number of serving instances', async () => {
  const down = await approvedRun('demo.replicas.scale', 'catalog', { replicas: 2 });
  assert.equal(down.plan.state, 'succeeded', JSON.stringify(down.plan));
  assert.deepEqual(down.plan.result.state.running, ['catalog-1', 'catalog-2']);
  assert.equal(down.plan.result.state.upstreams.length, 2);
  assert.equal(down.plan.verification.expected_instances, 2);
  assert.equal(down.plan.verification.instances, 2, JSON.stringify(down.plan.verification));

  const up = await approvedRun('demo.replicas.scale', 'catalog', { replicas: 3 });
  assert.equal(up.plan.state, 'succeeded', JSON.stringify(up.plan));
  assert.equal(up.plan.verification.instances, 3, JSON.stringify(up.plan.verification));
});

test('a feature flag that breaks its service fails verification; switching it off passes', async () => {
  const on = await approvedRun('demo.feature_flag.set', 'checkout-v2', { enabled: 1 });
  assert.equal(on.plan.state, 'verification_failed', JSON.stringify(on.plan));
  assert.equal(on.plan.result.flags['checkout-v2'], true, 'the step itself ran');
  assert.equal(on.plan.verification.service, 'orders');
  assert.equal(on.plan.verification.passed, false);
  assert.match(on.plan.verification.detail, /error ratio .* exceeds 5%/);
  assert.equal(on.plan.params.enabled, 1);

  const off = await approvedRun('demo.feature_flag.set', 'checkout-v2', { enabled: 0 });
  assert.equal(off.plan.state, 'succeeded', JSON.stringify(off.plan));
  assert.equal(off.plan.result.flags['checkout-v2'], false);
  assert.ok(off.plan.verification.errors / off.plan.verification.requests <= 0.05, JSON.stringify(off.plan.verification));
});

test('rollback returns orders to the earlier release, verified on that release only', async () => {
  const deployed = await postJson(`${DEMO_CONTROL}/releases/orders`, { version: '0.2.0' });
  assert.equal(deployed.status, 200, await deployed.text());
  const done = await approvedRun('demo.release.rollback', 'orders');
  assert.equal(done.plan.state, 'succeeded', JSON.stringify(done.plan));
  assert.deepEqual([done.plan.result.from_version, done.plan.result.to_version], ['0.2.0', '0.1.0']);
  const v = done.plan.verification;
  assert.equal(v.expected_version, '0.1.0');
  assert.equal(v.other_version_requests, 0, JSON.stringify(v));
  assert.ok(v.requests > 0 && v.passed, JSON.stringify(v));
  assert.equal((await (await fetch(`${DEMO_CONTROL}/releases/orders`)).json()).version, '0.1.0');
});

test('operators need an approval; one open plan per target; reject needs a comment; only proposer or admin can cancel', async () => {
  const p = await propose(operator, 'demo.faults.clear', 'gateway');
  assert.equal(p.status, 200, JSON.stringify(p.body));
  assert.equal(p.body.state, 'proposed', 'operators cannot run actions themselves');
  assert.equal(p.body.policy.requires_approval, true);
  const id = p.body.id;

  assert.equal((await operator.mutate(`${DEMO}/actions/${id}/approve`, {})).status, 403, 'operators cannot approve, not even their own');
  assert.equal((await viewer.mutate(`${DEMO}/actions/${id}/cancel`, {})).status, 403, 'viewers cannot cancel');

  const second = await propose(admin, 'demo.faults.clear', 'gateway');
  assert.equal(second.status, 403, 'running it yourself does not skip the policy');
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

  // An admin's own plan is approved at once; an operator may not cancel it.
  const mine = await propose(admin, 'demo.faults.clear', 'gateway');
  assert.equal(mine.status, 200, JSON.stringify(mine.body));
  assert.equal(mine.body.state, 'approved');
  assert.equal((await operator.mutate(`${DEMO}/actions/${mine.body.id}/cancel`, {})).status, 403, "cannot cancel someone else's plan");
  assert.equal((await waitFinal(mine.body.id)).plan.state, 'succeeded');
});

test('approved fault injection is executed, verified and audited; clearing restores the service', async () => {
  const p = await propose(operator, 'demo.faults.inject', 'orders', { latency_ms: 20, error_rate: 0.3 });
  assert.equal(p.status, 200, JSON.stringify(p.body));
  assert.ok(p.body.policy.allowed);
  const approved = await admin.mutate(`${DEMO}/actions/${p.body.id}/approve`, { comment: 'go' });
  assert.equal(approved.status, 200, JSON.stringify(approved.body));
  assert.equal(approved.body.state, 'approved');
  assert.equal(approved.body.decided_by, 'admin@karbird.local');

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

test('database restart goes through the executor and the service recovers', { skip: process.env.KARBIRD_TEST_RESTART === '0' }, async () => {
  const p = await propose(operator, 'demo.restart_database', 'demo-postgres');
  assert.equal(p.status, 200, JSON.stringify(p.body));
  assert.equal((await admin.mutate(`${DEMO}/actions/${p.body.id}/approve`, {})).status, 200);
  const done = await waitFinal(p.body.id);
  assert.equal(done.plan.state, 'succeeded', JSON.stringify(done.plan));
  assert.equal(done.plan.result.container, 'karbird-demo-postgres-1');
});
