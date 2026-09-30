// The personal account: profile, onboarding state, password change, sessions, and projects the
// user creates, renames or leaves. Uses fresh sign-ups only.
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { API, ORIGIN, Session, bearer, hex, scoped } from './lib.mjs';

const PASSWORD = `account horse ${randomUUID().slice(0, 8)}`;

async function login(email, password) {
  const s = new Session();
  const r = await s.json('/api/v1/session/login', {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify({ email, password }),
  });
  return { s, status: r.status };
}

async function signup(label) {
  const s = new Session();
  const email = `account-${label}-${randomUUID()}@example.test`;
  const r = await s.json('/api/v1/session/register', {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify({ email, password: PASSWORD, project_name: `Account ${hex(3)}` }),
  });
  assert.equal(r.status, 201, JSON.stringify(r.body));
  return { s, email, body: r.body };
}

// Sign-up is rate-limited globally, so the file uses two accounts: `me` and `other`, a second
// user in another tenant (and later a viewer of `me`'s project).
let me;
let other;
before(async () => {
  [me, other] = await Promise.all([signup('me'), signup('other')]);
});

test('a new account starts with onboarding open and can skip it or reopen it', async () => {
  assert.equal(me.body.user.onboarding_done, false);
  assert.ok(Math.abs(Date.parse(me.body.user.created_at) - Date.now()) < 5 * 60_000, me.body.user.created_at);

  const done = await me.s.mutate('/api/v1/account', { onboarding_done: true }, 'PATCH');
  assert.equal(done.status, 200, JSON.stringify(done.body));
  assert.equal(done.body.onboarding_done, true);
  assert.equal((await me.s.json('/api/v1/session')).body.user.onboarding_done, true);
  const reopened = await me.s.mutate('/api/v1/account', { onboarding_done: false }, 'PATCH');
  assert.equal(reopened.body.onboarding_done, false);
});

test('the display name can be changed but not blanked, and email stays as is', async () => {
  const name = `Ada ${hex(2)}`;
  const r = await me.s.mutate('/api/v1/account', { display_name: `  ${name} ` }, 'PATCH');
  assert.equal(r.status, 200, JSON.stringify(r.body));
  assert.deepEqual([r.body.display_name, r.body.email], [name, me.email]);
  for (const body of [{}, { display_name: '   ' }, { display_name: 'x'.repeat(101) }]) {
    assert.equal((await me.s.mutate('/api/v1/account', body, 'PATCH')).status, 400, JSON.stringify(body));
  }
  const ignored = await me.s.mutate('/api/v1/account', { display_name: name, email: 'evil@example.test' }, 'PATCH');
  assert.equal(ignored.body.email, me.email, 'email is not a writable field');
  assert.equal((await me.s.json('/api/v1/session')).body.user.display_name, name);
});

test('sessions are listed with the current one marked, and others can be signed out', async () => {
  const second = await login(me.email, PASSWORD);
  const third = await login(me.email, PASSWORD);
  assert.equal(second.status, 200);
  const list = await me.s.json('/api/v1/account/sessions');
  assert.equal(list.status, 200);
  assert.ok(list.body.length >= 3, JSON.stringify(list.body));
  assert.equal(list.body.filter((x) => x.current).length, 1);
  for (const row of list.body) {
    assert.deepEqual(Object.keys(row).sort(), ['created_at', 'current', 'expires_at', 'id'], 'no secrets or client details');
  }

  const mine = (await second.s.json('/api/v1/account/sessions')).body.find((x) => x.current);
  assert.equal((await me.s.mutate(`/api/v1/account/sessions/${mine.id}`, null, 'DELETE')).status, 204);
  assert.equal((await second.s.json('/api/v1/session')).status, 401, 'the revoked session is signed out');
  assert.equal((await me.s.mutate(`/api/v1/account/sessions/${mine.id}`, null, 'DELETE')).status, 404);
  assert.equal((await me.s.mutate('/api/v1/account/sessions/not-a-uuid', null, 'DELETE')).status, 404);

  const others = await me.s.mutate('/api/v1/account/sessions/revoke-others', {});
  assert.equal(others.status, 200);
  assert.ok(others.body.revoked >= 1);
  assert.equal((await third.s.json('/api/v1/session')).status, 401);
  assert.equal((await me.s.json('/api/v1/session')).status, 200, 'the current session stays');
});

test("one user cannot see or end another user's sessions", async () => {
  const theirs = (await other.s.json('/api/v1/account/sessions')).body;
  assert.equal(theirs.length, 1);
  assert.equal((await me.s.mutate(`/api/v1/account/sessions/${theirs[0].id}`, null, 'DELETE')).status, 404);
  assert.equal((await other.s.json('/api/v1/session')).status, 200);
  const mine = (await me.s.json('/api/v1/account/sessions')).body.map((x) => x.id);
  assert.ok(!mine.includes(theirs[0].id));
});

test('changing the password needs the current one, follows the sign-up rules and signs out other sessions', async () => {
  const other = await login(me.email, PASSWORD);
  assert.equal(other.status, 200);
  const change = (current_password, new_password) => me.s.mutate('/api/v1/account/password', { current_password, new_password });

  const wrong = await change('not my password', 'a brand new passphrase');
  assert.equal(wrong.status, 400);
  assert.match(wrong.body.message, /current password is wrong/);
  assert.equal((await change(PASSWORD, 'short')).status, 400);
  assert.equal((await change(PASSWORD, me.email)).status, 400, 'not the email');
  assert.equal((await other.s.json('/api/v1/session')).status, 200, 'refused changes end nothing');

  const next = `${PASSWORD} v2`;
  assert.equal((await change(PASSWORD, next)).status, 204);
  assert.equal((await other.s.json('/api/v1/session')).status, 401, 'other sessions are signed out');
  assert.equal((await me.s.json('/api/v1/session')).status, 200, 'this one stays');
  assert.equal((await login(me.email, PASSWORD)).status, 401, 'the old password no longer works');
  assert.equal((await login(me.email, next)).status, 200);
  assert.equal((await change(next, PASSWORD)).status, 204, 'and back, for the rest of the file');
});

test('account mutations need a session, CSRF and Origin, never an API token', async () => {
  const anon = await fetch(`${API}/api/v1/account`, {
    method: 'PATCH',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify({ display_name: 'x' }),
  });
  assert.equal(anon.status, 401);
  assert.equal((await fetch(`${API}/api/v1/account/sessions`)).status, 401);
  const noCsrf = await me.s.json('/api/v1/account/password', {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify({ current_password: PASSWORD, new_password: 'another long passphrase' }),
  });
  assert.equal(noCsrf.status, 403);
  const evil = await me.s.json('/api/v1/projects', {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: 'https://evil.example', 'x-csrf-token': me.s.cookies.get('controlhub_csrf') },
    body: JSON.stringify({ name: 'Evil' }),
  });
  assert.equal(evil.status, 403);

  const project = me.body.projects[0];
  const token = await me.s.mutate(`${scoped(project.id, project.environments[0].id)}/tokens`, { kind: 'api', name: `acc-${hex(3)}` });
  assert.equal(token.status, 200, JSON.stringify(token.body));
  const t = token.body.secret;
  const json = { 'content-type': 'application/json' };
  assert.equal((await bearer(t, '/api/v1/account/sessions')).status, 403);
  assert.equal((await bearer(t, '/api/v1/account', { method: 'PATCH', headers: json, body: '{"display_name":"x"}' })).status, 403);
  assert.equal((await bearer(t, '/api/v1/projects', { method: 'POST', headers: json, body: '{"name":"x"}' })).status, 403);
  assert.equal((await bearer(t, `/api/v1/projects/${project.id}/leave`, { method: 'POST', headers: json, body: '{}' })).status, 403);
});

test('a user creates projects they own, renames them, and is capped at 20', async () => {
  const name = `Side project ${hex(3)}`;
  const created = await me.s.mutate('/api/v1/projects', { name });
  assert.equal(created.status, 201, JSON.stringify(created.body));
  assert.deepEqual([created.body.name, created.body.role], [name, 'owner']);
  assert.deepEqual(created.body.environments.map((e) => [e.name, e.is_demo]), [['production', false]]);
  const projects = (await me.s.json('/api/v1/session')).body.projects;
  assert.ok(projects.some((p) => p.id === created.body.id));
  assert.equal((await me.s.mutate('/api/v1/projects', { name: '  ' })).status, 400);

  const path = `/api/v1/projects/${created.body.id}`;
  const renamed = `${name} renamed`;
  assert.equal((await me.s.mutate(path, { name: renamed }, 'PATCH')).status, 204);
  assert.equal((await me.s.json('/api/v1/session')).body.projects.find((p) => p.id === created.body.id).name, renamed);
  assert.equal((await me.s.mutate(path, { name: '' }, 'PATCH')).status, 400);

  // Another tenant cannot rename or leave it.
  assert.equal((await other.s.mutate(path, { name: 'Hijacked' }, 'PATCH')).status, 404);
  assert.equal((await other.s.mutate(`${path}/leave`, {})).status, 404);

  const owned = (await me.s.json('/api/v1/session')).body.projects.filter((p) => p.role === 'owner').length;
  for (let i = owned; i < 20; i++) {
    const r = await me.s.mutate('/api/v1/projects', { name: `Cap ${i} ${hex(2)}` });
    assert.equal(r.status, 201, JSON.stringify(r.body));
  }
  const capped = await me.s.mutate('/api/v1/projects', { name: `One too many ${hex(2)}` });
  assert.equal(capped.status, 409);
  assert.match(capped.body.message, /maximum number of projects/);

  const audit = await me.s.json(`${scoped(created.body.id, created.body.environments[0].id)}/audit`);
  const actions = new Set(audit.body.map((e) => e.action));
  assert.ok(actions.has('project.create') && actions.has('project.rename'), [...actions].join(', '));
});

test('renaming needs project.manage', async () => {
  const project = me.body.projects[0];
  const roles = (await me.s.json(`/api/v1/projects/${project.id}/roles`)).body;
  const inv = await me.s.mutate(`/api/v1/projects/${project.id}/invites`, { role_id: roles.find((r) => r.builtin === 'viewer').id, max_uses: 1 });
  assert.equal(inv.status, 201, JSON.stringify(inv.body));
  const viewer = other.s;
  assert.equal((await viewer.mutate('/api/v1/invites/accept', { token: inv.body.secret })).status, 200);
  assert.equal((await viewer.mutate(`/api/v1/projects/${project.id}`, { name: 'Viewer rename' }, 'PATCH')).status, 403);
  assert.equal((await viewer.mutate(`/api/v1/projects/${project.id}/leave`, {})).status, 204, 'a non-owner may leave');
  assert.ok(!(await viewer.json('/api/v1/session')).body.projects.some((p) => p.id === project.id));
});
