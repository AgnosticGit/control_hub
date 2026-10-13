// Project roles and members: templates, individual permission sets, anti-escalation rules,
// CSRF, tenant isolation and audit. Uses only the member-a/member-b fixture accounts (their
// permissions change here) and role names starting with "[test] " (removed after the run).
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { API, DEMO, ORIGIN, Session, bearer, hex, user } from './lib.mjs';

const ROLES = '/api/v1/projects/demo/roles';
const MEMBERS = '/api/v1/projects/demo/members';
const ALL = [
  'telemetry.view',
  'incidents.manage',
  'tokens.view',
  'tokens.manage',
  'actions.view',
  'actions.propose',
  'actions.approve',
  'members.view',
  'members.manage',
  'roles.manage',
  'project.manage',
];
const roleName = (label) => `[test] ${label} ${hex(3)}`;

const admin = new Session(); // owner of demo
const a = new Session();
const b = new Session();
let ids; // user ids by email
let builtin; // role ids by built-in key

async function sessionProject(s, id = 'demo') {
  const r = await s.json('/api/v1/session');
  assert.equal(r.status, 200);
  return r.body.projects.find((p) => p.id === id);
}

async function setRole(actor, email, roleId) {
  return actor.mutate(`${MEMBERS}/${ids[email]}`, { role_id: roleId }, 'PATCH');
}

async function createRole(label, permissions) {
  const r = await admin.mutate(ROLES, { name: roleName(label), permissions });
  assert.equal(r.status, 200, JSON.stringify(r.body));
  return r.body;
}

before(async () => {
  await admin.login('admin@karbird.local');
  await a.login('member-a@karbird.local');
  await b.login('member-b@karbird.local');
  const members = await admin.json(MEMBERS);
  assert.equal(members.status, 200, JSON.stringify(members.body));
  ids = Object.fromEntries(members.body.map((m) => [m.email, m.user_id]));
  const roles = await admin.json(ROLES);
  builtin = Object.fromEntries(roles.body.filter((r) => r.builtin).map((r) => [r.builtin, r.id]));
  assert.deepEqual(Object.keys(builtin).sort(), ['admin', 'operator', 'owner', 'viewer']);
});

after(async () => {
  // Leave the fixture members as viewers so a rerun starts from the same state.
  for (const email of ['member-a@karbird.local', 'member-b@karbird.local']) {
    await setRole(admin, email, builtin.viewer);
  }
});

test('the permission catalog needs a session and lists every permission once', async () => {
  const anon = await fetch(`${API}/api/v1/permissions`);
  assert.equal(anon.status, 401);
  const r = await b.json('/api/v1/permissions');
  assert.equal(r.status, 200);
  assert.deepEqual(r.body.map((p) => p.id), ALL);
  assert.ok(r.body.every((p) => p.label && p.group));
});

test('/session carries the role and effective permissions per project', async () => {
  const owner = await sessionProject(admin);
  assert.deepEqual([owner.role, owner.role_name], ['owner', 'Owner']);
  assert.deepEqual(owner.permissions, ALL);
  const viewer = await sessionProject(b);
  assert.deepEqual([viewer.role, viewer.role_name], ['viewer', 'Viewer']);
  assert.deepEqual(viewer.permissions, ['telemetry.view', 'actions.view', 'members.view']);
});

test('built-in roles are listed with holders and cannot be changed or deleted', async () => {
  const roles = await b.json(ROLES);
  assert.equal(roles.status, 200, 'members.view is enough to list roles');
  const owner = roles.body.find((r) => r.builtin === 'owner');
  assert.ok(owner.holders >= 1);
  assert.deepEqual(roles.body.slice(0, 4).map((r) => r.builtin), ['owner', 'admin', 'operator', 'viewer']);
  const patch = await admin.mutate(`${ROLES}/${builtin.viewer}`, { name: 'Viewer', permissions: ALL }, 'PATCH');
  assert.equal(patch.status, 403);
  assert.equal((await admin.mutate(`${ROLES}/${builtin.viewer}`, null, 'DELETE')).status, 403);
  const after = await admin.json(ROLES);
  assert.deepEqual(after.body.find((r) => r.builtin === 'viewer').permissions, ['telemetry.view', 'actions.view', 'members.view']);
});

test('a template grants its permissions and editing it changes every holder at once', async () => {
  assert.equal((await a.json(`${DEMO}/tokens`)).status, 403, 'viewer cannot list tokens');
  const manager = await createRole('Manager', ['telemetry.view', 'tokens.view', 'actions.view']);
  assert.deepEqual([manager.builtin, manager.holders], [null, 0]);

  const assigned = await setRole(admin, 'member-a@karbird.local', manager.id);
  assert.equal(assigned.status, 200, JSON.stringify(assigned.body));
  assert.deepEqual([assigned.body.role, assigned.body.role_name], ['custom', manager.name]);
  assert.equal((await a.json(`${DEMO}/tokens`)).status, 200, 'tokens.view now allows listing');
  assert.equal((await a.mutate(`${DEMO}/tokens`, { kind: 'api', name: 'nope' })).status, 403, 'but not creating');
  const project = await sessionProject(a);
  assert.deepEqual([project.role, project.role_name], ['custom', manager.name]);

  const edited = await admin.mutate(`${ROLES}/${manager.id}`, { name: manager.name, permissions: ['telemetry.view'] }, 'PATCH');
  assert.equal(edited.status, 200, JSON.stringify(edited.body));
  assert.equal(edited.body.holders, 1);
  assert.equal((await a.json(`${DEMO}/tokens`)).status, 403, 'removed from the template, removed from the member');
  assert.equal((await a.json(`${DEMO}/services`)).status, 200);
  assert.equal((await a.json(MEMBERS)).status, 403, 'members.view is no longer in the template either');

  // Held templates cannot be deleted; once nobody holds it, it can.
  const held = await admin.mutate(`${ROLES}/${manager.id}`, null, 'DELETE');
  assert.equal(held.status, 409);
  assert.equal((await setRole(admin, 'member-a@karbird.local', builtin.viewer)).status, 200);
  assert.equal((await admin.mutate(`${ROLES}/${manager.id}`, null, 'DELETE')).status, 204);
  assert.equal((await admin.mutate(`${ROLES}/${manager.id}`, null, 'DELETE')).status, 404);
});

test('an individual set replaces the role and remembers what it was based on', async () => {
  const r = await admin.mutate(
    `${MEMBERS}/${ids['member-a@karbird.local']}`,
    { permissions: ['telemetry.view', 'actions.propose'], based_on: builtin.viewer },
    'PATCH',
  );
  assert.equal(r.status, 200, JSON.stringify(r.body));
  assert.deepEqual([r.body.role, r.body.role_id, r.body.based_on], ['individual', null, builtin.viewer]);
  assert.deepEqual(r.body.permissions, ['telemetry.view', 'actions.view', 'actions.propose'], 'implied actions.view added');
  const project = await sessionProject(a);
  assert.deepEqual([project.role, project.role_name], ['individual', 'Individual']);
  assert.equal((await a.json(`${DEMO}/actions`)).status, 200);
});

test('role and member input is validated', async () => {
  const taken = roleName('Dup');
  assert.equal((await admin.mutate(ROLES, { name: taken, permissions: [] })).status, 200);
  assert.equal((await admin.mutate(ROLES, { name: taken.toUpperCase(), permissions: [] })).status, 409, 'names are case-insensitive');
  assert.equal((await admin.mutate(ROLES, { name: 'admin', permissions: [] })).status, 409, 'built-in names are taken');
  for (const body of [
    { name: 'Individual', permissions: [] },
    { name: '   ', permissions: [] },
    { name: 'x'.repeat(65), permissions: [] },
    { name: roleName('Bad'), permissions: ['root'] },
    { name: roleName('Bad'), permissions: ['tokens.*'] },
  ]) {
    assert.equal((await admin.mutate(ROLES, body)).status, 400, JSON.stringify(body));
  }
  const member = `${MEMBERS}/${ids['member-b@karbird.local']}`;
  assert.equal((await admin.mutate(member, {}, 'PATCH')).status, 400, 'neither role nor permissions');
  assert.equal((await admin.mutate(member, { role_id: builtin.viewer, permissions: [] }, 'PATCH')).status, 400, 'both');
  assert.equal((await admin.mutate(member, { permissions: ['owner'] }, 'PATCH')).status, 400);
  assert.equal((await admin.mutate(member, { role_id: 'not-a-uuid' }, 'PATCH')).status, 404);
  assert.equal((await admin.mutate(`${MEMBERS}/00000000-0000-0000-0000-000000000000`, { role_id: builtin.viewer }, 'PATCH')).status, 404);
});

test('a member manager cannot escalate, touch owners, stronger members or themselves', async () => {
  const lead = await createRole('Team lead', ['telemetry.view', 'actions.view', 'members.view', 'members.manage']);
  assert.equal((await setRole(admin, 'member-a@karbird.local', lead.id)).status, 200);
  assert.equal((await setRole(admin, 'member-b@karbird.local', builtin.viewer)).status, 200);
  const bId = ids['member-b@karbird.local'];

  const toAdmin = await setRole(a, 'member-b@karbird.local', builtin.admin);
  assert.equal(toAdmin.status, 403);
  assert.match(toAdmin.body.message, /permissions you have yourself/);
  const extra = await a.mutate(`${MEMBERS}/${bId}`, { permissions: ['telemetry.view', 'tokens.view'] }, 'PATCH');
  assert.equal(extra.status, 403, 'individual sets are limited too');
  const ok = await a.mutate(`${MEMBERS}/${bId}`, { permissions: ['telemetry.view'] }, 'PATCH');
  assert.equal(ok.status, 200, 'within their own permissions it works');

  const self = await a.mutate(`${MEMBERS}/${ids['member-a@karbird.local']}`, { permissions: ALL }, 'PATCH');
  assert.equal(self.status, 403);
  assert.match(self.body.message, /your own membership/);
  const owner = await a.mutate(`${MEMBERS}/${ids['admin@karbird.local']}`, { permissions: [] }, 'PATCH');
  assert.equal(owner.status, 403);
  assert.equal((await a.mutate(`${MEMBERS}/${ids['admin@karbird.local']}`, null, 'DELETE')).status, 403);
  assert.equal((await a.mutate(ROLES, { name: roleName('Sneaky'), permissions: [] })).status, 403, 'no roles.manage');

  // Owners are only changed by owners; the Owner role is only given by owners.
  assert.equal((await setRole(admin, 'member-b@karbird.local', builtin.owner)).status, 200);
  const touchOwner = await setRole(a, 'member-b@karbird.local', builtin.viewer);
  assert.equal(touchOwner.status, 403);
  assert.match(touchOwner.body.message, /owner/);
  assert.equal((await setRole(admin, 'member-b@karbird.local', builtin.viewer)).status, 200);

  // An owner cannot change their own membership either (so an owner is never lost by accident).
  const ownSelf = await setRole(admin, 'admin@karbird.local', builtin.viewer);
  assert.equal(ownSelf.status, 403);
  assert.equal((await sessionProject(admin)).role, 'owner');
});

test('removing a member ends their access to the project', async () => {
  const removed = await admin.mutate(`${MEMBERS}/${ids['member-b@karbird.local']}`, null, 'DELETE');
  assert.equal(removed.status, 204);
  assert.equal((await b.json(`${DEMO}/services`)).status, 404, 'the project now looks absent');
  assert.equal(await sessionProject(b), undefined);
  assert.equal((await admin.mutate(`${MEMBERS}/${ids['member-b@karbird.local']}`, null, 'DELETE')).status, 404);
  // Only the membership is gone; the account and its session stay (the next fixture run re-adds it).
  const me = await fetch(`${API}/api/v1/session`, { headers: { cookie: b.cookieHeader() } });
  assert.equal(me.status, 200);
});

test('mutations need CSRF and an allowed Origin; API tokens never manage members', async () => {
  const path = `${MEMBERS}/${ids['member-a@karbird.local']}`;
  const body = JSON.stringify({ role_id: builtin.viewer });
  const noCsrf = await admin.json(path, { method: 'PATCH', headers: { 'content-type': 'application/json', origin: ORIGIN }, body });
  assert.equal(noCsrf.status, 403);
  const evil = await admin.json(path, {
    method: 'PATCH',
    headers: { 'content-type': 'application/json', origin: 'https://evil.example', 'x-csrf-token': admin.cookies.get('karbird_csrf') },
    body,
  });
  assert.equal(evil.status, 403);
  const noCsrfRole = await admin.json(ROLES, {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify({ name: roleName('CSRF'), permissions: [] }),
  });
  assert.equal(noCsrfRole.status, 403);

  const token = await admin.mutate(`${DEMO}/tokens`, { kind: 'api', name: `roles-${hex(3)}` });
  assert.equal(token.status, 200);
  const secret = token.body.secret;
  assert.equal((await bearer(secret, MEMBERS)).status, 403, 'API tokens cannot even list members');
  assert.equal((await bearer(secret, path, { method: 'DELETE' })).status, 403);
  const session = await bearer(secret, '/api/v1/session');
  assert.deepEqual(session.body.projects[0].permissions, ['telemetry.view', 'actions.view'], 'tokens are capped to reads');
  await admin.mutate(`${DEMO}/tokens/${token.body.token.id}/revoke`, {});
});

test('another tenant cannot see or change demo members and roles, and ids do not cross projects', async () => {
  const iso = new Session();
  await iso.login('isolation@karbird.local');
  assert.equal((await iso.json(MEMBERS)).status, 404);
  assert.equal((await iso.json(ROLES)).status, 404);
  assert.equal((await iso.mutate(`${MEMBERS}/${ids['member-a@karbird.local']}`, { permissions: [] }, 'PATCH')).status, 404);
  assert.equal((await iso.mutate(ROLES, { name: roleName('Intruder'), permissions: [] })).status, 404);
  assert.equal((await admin.json('/api/v1/projects/isolation/members')).status, 404);

  const isoRoles = await iso.json('/api/v1/projects/isolation/roles');
  assert.equal(isoRoles.status, 200);
  const foreignRole = isoRoles.body.find((r) => r.builtin === 'viewer').id;
  const cross = await setRole(admin, 'member-a@karbird.local', foreignRole);
  assert.equal(cross.status, 404, "a role of another project is unknown here");
  assert.ok(!(await iso.json('/api/v1/projects/isolation/members')).body.some((m) => m.email === user('member-a@karbird.local').email));
});

test('role and member changes, including refusals, are audited', async () => {
  const audit = await admin.json(`${DEMO}/audit`);
  assert.equal(audit.status, 200);
  const actions = new Set(audit.body.map((e) => `${e.action}:${e.outcome}`));
  for (const expected of ['role.create:succeeded', 'role.update:succeeded', 'role.delete:succeeded', 'member.update:succeeded', 'member.update:denied', 'member.remove:succeeded']) {
    assert.ok(actions.has(expected), `${expected} missing from ${[...actions].join(', ')}`);
  }
});
