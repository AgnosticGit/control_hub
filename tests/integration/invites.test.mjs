// Invite links: creation limits, preview, accept, use limits, revocation, the creator re-check,
// CSRF/Origin, API-token refusal, tenant isolation, audit and the last-owner guard. Every account
// here is a fresh sign-up with its own project, so nothing is shared with other test files.
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { API, ORIGIN, Session, bearer, hex, scoped } from './lib.mjs';

const PASSWORD = `invite horse ${randomUUID().slice(0, 8)}`;

async function signup(label) {
  const s = new Session();
  const email = `invite-${label}-${randomUUID()}@example.test`;
  const r = await s.json('/api/v1/session/register', {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify({ email, password: PASSWORD, project_name: `Invites ${label} ${hex(3)}` }),
  });
  assert.equal(r.status, 201, JSON.stringify(r.body));
  const project = r.body.projects[0];
  return { s, email, id: r.body.user.id, project, env: project.environments[0] };
}

const preview = (s, token, origin = ORIGIN) =>
  s.json('/api/v1/invites/preview', {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin, 'x-csrf-token': s.cookies.get('controlhub_csrf') ?? '' },
    body: JSON.stringify({ token }),
  });
const accept = (who, token) => who.s.mutate('/api/v1/invites/accept', { token });

let owner; // owns the project the invites are for
let joiner;
let third;
let stranger; // another tenant, created in the CSRF test
let invites; // `/api/v1/projects/<id>/invites`
let roles; // built-in role ids by key

before(async () => {
  [owner, joiner, third] = await Promise.all([signup('owner'), signup('joiner'), signup('third')]);
  invites = `/api/v1/projects/${owner.project.id}/invites`;
  const r = await owner.s.json(`/api/v1/projects/${owner.project.id}/roles`);
  roles = Object.fromEntries(r.body.filter((x) => x.builtin).map((x) => [x.builtin, x.id]));
});

async function createInvite(actor, body) {
  const r = await actor.s.mutate(invites, body);
  assert.equal(r.status, 201, JSON.stringify(r.body));
  return r.body;
}

test('an invite link shows the project to anyone with it and adds the signed-in user once', async () => {
  const created = await createInvite(owner, { role_id: roles.operator, max_uses: 5 });
  assert.match(created.secret, /^chv_[A-Za-z0-9_-]+$/);
  assert.ok(created.secret.startsWith(created.invite.prefix), 'prefix is the non-secret start');
  assert.deepEqual([created.invite.role_name, created.invite.uses, created.invite.max_uses], ['Operator', 0, 5]);
  const days = (Date.parse(created.invite.expires_at) - Date.now()) / 86_400_000;
  assert.ok(days > 6.9 && days <= 7, `default expiry is a week, got ${days}`);

  const listed = await owner.s.json(invites);
  assert.equal(listed.status, 200);
  assert.ok(listed.body.some((i) => i.id === created.invite.id));
  assert.ok(!JSON.stringify(listed.body).includes(created.secret), 'the secret is never listed');

  const anon = await preview(new Session(), created.secret);
  assert.equal(anon.status, 200, JSON.stringify(anon.body));
  assert.deepEqual([anon.body.project_name, anon.body.role_name, anon.body.signed_in, anon.body.already_member], [owner.project.name, 'Operator', false, false]);
  assert.equal(anon.body.invited_by, owner.email.split('@')[0]);
  assert.equal((await preview(joiner.s, created.secret)).body.signed_in, true);
  const cookieNoCsrf = await joiner.s.json('/api/v1/invites/preview', {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify({ token: created.secret }),
  });
  assert.equal(cookieNoCsrf.status, 403, 'a cookie session without the CSRF header is refused, not treated as anonymous');

  const joined = await accept(joiner, created.secret);
  assert.equal(joined.status, 200, JSON.stringify(joined.body));
  assert.deepEqual(joined.body, { project_id: owner.project.id, environment_id: owner.env.id, already_member: false });
  const session = await joiner.s.json('/api/v1/session');
  const project = session.body.projects.find((p) => p.id === owner.project.id);
  assert.deepEqual([project.role, project.role_name], ['operator', 'Operator']);
  assert.equal((await joiner.s.json(`${scoped(owner.project.id, owner.env.id)}/services`)).status, 200, 'can read telemetry now');

  const again = await accept(joiner, created.secret);
  assert.equal(again.status, 200);
  assert.equal(again.body.already_member, true);
  assert.equal((await preview(joiner.s, created.secret)).body.already_member, true);
  const after = (await owner.s.json(invites)).body.find((i) => i.id === created.invite.id);
  assert.equal(after.uses, 1, 'accepting as an existing member spends no use');
});

test('unknown secrets are 404; used-up and revoked links are 410', async () => {
  const s = new Session();
  for (const token of [`chv_${hex(24)}`, 'chv_<script>', 'cha_not_an_invite', '']) {
    assert.equal((await preview(s, token)).status, 404, token);
  }
  assert.equal((await preview(s, `chv_${hex(24)}`, 'https://evil.example')).status, 403, 'preview checks Origin');

  const single = await createInvite(owner, { role_id: roles.viewer, max_uses: 1, expires_in_days: 1 });
  assert.equal((await accept(third, single.secret)).status, 200);
  const late = new Session();
  const used = await preview(late, single.secret);
  assert.equal(used.status, 410);
  assert.equal(used.body.code, 'gone');
  assert.ok(!(await owner.s.json(invites)).body.some((i) => i.id === single.invite.id), 'used-up links leave the active list');

  const revokable = await createInvite(owner, { role_id: roles.viewer });
  assert.equal((await owner.s.mutate(`${invites}/${revokable.invite.id}`, null, 'DELETE')).status, 204);
  assert.equal((await owner.s.mutate(`${invites}/${revokable.invite.id}`, null, 'DELETE')).status, 404);
  assert.equal((await preview(late, revokable.secret)).status, 410);
  assert.equal((await accept(joiner, revokable.secret)).status, 410);
});

test('invite input is validated', async () => {
  for (const body of [
    { role_id: roles.viewer, expires_in_days: 0 },
    { role_id: roles.viewer, expires_in_days: 31 },
    { role_id: roles.viewer, max_uses: 0 },
    { role_id: roles.viewer, max_uses: 1001 },
  ]) {
    assert.equal((await owner.s.mutate(invites, body)).status, 400, JSON.stringify(body));
  }
  assert.equal((await owner.s.mutate(invites, { role_id: 'not-a-uuid' })).status, 404);
  assert.equal((await owner.s.mutate(invites, { role_id: randomUUID() })).status, 404);
});

test('only owners invite with the Owner role, nobody invites above themselves, and a demoted creator voids the link', async () => {
  const toAdmin = await createInvite(owner, { role_id: roles.admin, max_uses: 1 });
  const admin = await signup('admin');
  assert.equal((await accept(admin, toAdmin.secret)).status, 200);

  const ownerInvite = await admin.s.mutate(invites, { role_id: roles.owner });
  assert.equal(ownerInvite.status, 403);
  assert.match(ownerInvite.body.message, /only an owner/);
  assert.equal((await admin.s.mutate(invites, { role_id: roles.admin })).status, 201, 'an admin may invite admins');

  const leadRole = await owner.s.mutate(`/api/v1/projects/${owner.project.id}/roles`, {
    name: `[test] Lead ${hex(3)}`,
    permissions: ['telemetry.view', 'actions.view', 'members.view', 'members.manage'],
  });
  assert.equal(leadRole.status, 200, JSON.stringify(leadRole.body));
  const lead = await signup('lead');
  assert.equal((await accept(lead, (await createInvite(owner, { role_id: leadRole.body.id, max_uses: 1 })).secret)).status, 200);
  const above = await lead.s.mutate(invites, { role_id: roles.operator });
  assert.equal(above.status, 403, 'Operator has tokens.view and actions.propose, which the lead lacks');
  assert.match(above.body.message, /permissions you have yourself/);

  const byLead = await createInvite(lead, { role_id: roles.viewer });
  assert.equal((await preview(new Session(), byLead.secret)).status, 200);
  const demote = await owner.s.mutate(`/api/v1/projects/${owner.project.id}/members/${lead.id}`, { role_id: roles.viewer }, 'PATCH');
  assert.equal(demote.status, 200, JSON.stringify(demote.body));
  assert.equal((await preview(new Session(), byLead.secret)).status, 410, 'the creator can no longer grant it');
  assert.equal((await accept(third, byLead.secret)).status, 410);
  assert.equal((await lead.s.json(invites)).status, 403, 'and cannot list invites any more');
});

test('invite mutations need CSRF and Origin; API tokens and other tenants are refused', async () => {
  const body = JSON.stringify({ role_id: roles.viewer });
  const noCsrf = await owner.s.json(invites, { method: 'POST', headers: { 'content-type': 'application/json', origin: ORIGIN }, body });
  assert.equal(noCsrf.status, 403);
  const evil = await owner.s.json(invites, {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: 'https://evil.example', 'x-csrf-token': owner.s.cookies.get('controlhub_csrf') },
    body,
  });
  assert.equal(evil.status, 403);

  const link = await createInvite(owner, { role_id: roles.viewer });
  stranger = await signup('stranger');
  const noCsrfAccept = await stranger.s.json('/api/v1/invites/accept', {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify({ token: link.secret }),
  });
  assert.equal(noCsrfAccept.status, 403);
  const anonAccept = await fetch(`${API}/api/v1/invites/accept`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify({ token: link.secret }),
  });
  assert.equal(anonAccept.status, 401);

  const token = await owner.s.mutate(`${scoped(owner.project.id, owner.env.id)}/tokens`, { kind: 'api', name: `inv-${hex(3)}` });
  assert.equal(token.status, 200, JSON.stringify(token.body));
  const secret = token.body.secret;
  assert.equal((await bearer(secret, invites)).status, 403, 'API tokens cannot list invites');
  const viaToken = await bearer(secret, invites, { method: 'POST', headers: { 'content-type': 'application/json' }, body });
  assert.equal(viaToken.status, 403);
  const acceptViaToken = await bearer(secret, '/api/v1/invites/accept', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ token: link.secret }),
  });
  assert.equal(acceptViaToken.status, 403);

  // The stranger's own project and the owner's are invisible to each other's invite endpoints.
  assert.equal((await stranger.s.json(invites)).status, 404);
  assert.equal((await stranger.s.mutate(invites, { role_id: roles.viewer })).status, 404);
  assert.equal((await stranger.s.mutate(`${invites}/${link.invite.id}`, null, 'DELETE')).status, 404);
  const foreignRoles = await stranger.s.json(`/api/v1/projects/${stranger.project.id}/roles`);
  const foreignViewer = foreignRoles.body.find((r) => r.builtin === 'viewer').id;
  assert.equal((await owner.s.mutate(invites, { role_id: foreignViewer })).status, 404, 'a role of another project is unknown here');
  const strangerInvites = `/api/v1/projects/${stranger.project.id}/invites`;
  assert.equal((await owner.s.mutate(`${strangerInvites}/${link.invite.id}`, null, 'DELETE')).status, 404, 'invite ids do not cross projects');
  assert.equal((await preview(new Session(), link.secret)).status, 200, 'the link survived both attempts');
});

test('the last owner cannot leave until someone else is an owner', async () => {
  const leave = (who) => who.s.mutate(`/api/v1/projects/${owner.project.id}/leave`, {});
  const blocked = await leave(owner);
  assert.equal(blocked.status, 409);
  assert.match(blocked.body.message, /last owner/);

  // Sign-up is rate-limited globally: the stranger of the previous test becomes the successor.
  const successor = stranger ?? (await signup('successor'));
  const ownerLink = await createInvite(owner, { role_id: roles.owner, max_uses: 1 });
  assert.equal((await accept(successor, ownerLink.secret)).status, 200);
  assert.equal((await leave(owner)).status, 204);
  assert.ok(!(await owner.s.json('/api/v1/session')).body.projects.some((p) => p.id === owner.project.id));
  assert.equal((await leave(owner)).status, 404, 'no longer a member');
  assert.equal((await leave(successor)).status, 409, 'now the successor is the last owner');

  const audit = await successor.s.json(`${scoped(owner.project.id, owner.env.id)}/audit`);
  assert.equal(audit.status, 200);
  const actions = new Set(audit.body.map((e) => `${e.action}:${e.outcome}`));
  for (const expected of ['invite.create:succeeded', 'invite.create:denied', 'invite.revoke:succeeded', 'invite.accept:succeeded', 'member.leave:succeeded']) {
    assert.ok(actions.has(expected), `${expected} missing from ${[...actions].join(', ')}`);
  }
});
