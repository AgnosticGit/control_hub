import { test } from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { API, DEMO, ISO, Session, eventually } from './lib.mjs';

const dockerId = 'a'.repeat(64);

test('one-container Control enrolls once, requires owner confirmation, dispatches one approved action and revokes', async () => {
  const admin = new Session();
  await admin.login('admin@karbird.local');
  const created = await admin.mutate(`${DEMO}/control/installations`, { name: `control-${randomUUID().slice(0, 8)}` });
  assert.equal(created.status, 200, JSON.stringify(created.body));
  const id = created.body.installation.id;
  const code = created.body.enrollment_code;
  try {
    const enroll = await fetch(`${API}/api/v1/control/enroll`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ enrollment_code: code }) });
    assert.equal(enroll.status, 200);
    const { credential } = await enroll.json();
    assert.ok(credential.startsWith('chcc_'));
    const reused = await fetch(`${API}/api/v1/control/enroll`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ enrollment_code: code }) });
    assert.equal(reused.status, 401);
    const auth = { authorization: `Bearer ${credential}`, 'content-type': 'application/json' };
    const report = await fetch(`${API}/api/v1/control/report`, { method: 'POST', headers: auth,
      body: JSON.stringify({ container_id: dockerId, container_name: 'control-test', state: 'running' }) });
    assert.equal(report.status, 200);
    const before = await admin.mutate(`${DEMO}/actions`, { action_type: 'container.restart', target: id, params: {}, reason: 'integration control before confirmation' });
    assert.equal(before.status, 404);
    const foreign = await admin.mutate(`${ISO}/control/installations/${id}/confirm`, { container_id: dockerId });
    assert.equal(foreign.status, 404);
    const wrong = await admin.mutate(`${DEMO}/control/installations/${id}/confirm`, { container_id: 'b'.repeat(64) });
    assert.equal(wrong.status, 409);
    const confirmed = await admin.mutate(`${DEMO}/control/installations/${id}/confirm`, { container_id: dockerId });
    assert.equal(confirmed.status, 200, JSON.stringify(confirmed.body));
    const changedIdentity = await fetch(`${API}/api/v1/control/report`, { method: 'POST', headers: auth,
      body: JSON.stringify({ container_id: 'b'.repeat(64), container_name: 'different-container', state: 'running' }) });
    assert.equal(changedIdentity.status, 409, 'a confirmed installation cannot silently switch containers');
    for (const [verb, result] of [
      ['restart', { succeeded: true, observed_state: 'running', previous_started_at: 'a', started_at: 'b' }],
      ['stop', { succeeded: true, observed_state: 'stopped' }],
      ['start', { succeeded: true, observed_state: 'running' }],
    ]) {
      const proposal = await admin.mutate(`${DEMO}/actions`, { action_type: `container.${verb}`, target: id, params: {}, reason: `integration Control ${verb}` });
      assert.equal(proposal.status, 200, JSON.stringify(proposal.body));
      const plan = proposal.body.id;
      let command;
      await eventually(async () => {
        const poll = await fetch(`${API}/api/v1/control/commands`, { headers: auth });
        assert.equal(poll.status, 200);
        command = await poll.json();
        assert.ok(command, 'executor dispatched the command');
      }, { timeoutMs: 30000, intervalMs: 500 });
      assert.equal(command.verb, verb);
      assert.equal(command.container_id, dockerId);
      const complete = await fetch(`${API}/api/v1/control/commands/${command.id}/complete`, { method: 'POST', headers: auth, body: JSON.stringify(result) });
      assert.equal(complete.status, 200);
      const duplicate = await fetch(`${API}/api/v1/control/commands/${command.id}/complete`, { method: 'POST', headers: auth, body: JSON.stringify(result) });
      assert.equal(duplicate.status, 409);
      await eventually(async () => {
        const detail = await admin.json(`${DEMO}/actions/${plan}`);
        assert.equal(detail.body.plan.state, 'succeeded', JSON.stringify(detail.body.plan));
        assert.equal(detail.body.plan.verification.passed, true);
      }, { timeoutMs: 30000, intervalMs: 500 });
    }
    const another = await fetch(`${API}/api/v1/control/commands`, { headers: auth });
    assert.equal(await another.json(), null, 'completed command is never redelivered');
    const revoked = await admin.mutate(`${DEMO}/control/installations/${id}/revoke`, {});
    assert.equal(revoked.status, 200);
    const denied = await fetch(`${API}/api/v1/control/commands`, { headers: auth });
    assert.equal(denied.status, 401);
  } finally {
    await admin.mutate(`${DEMO}/control/installations/${id}/revoke`, {}).catch(() => undefined);
  }
});
