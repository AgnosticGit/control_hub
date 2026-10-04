// karbird administration (/api/v1/admin/*): access per administrator role, project lifecycle,
// accounts, administrator grants and roles, CSRF, API tokens, isolation and audit. Uses only the
// system fixture accounts (superadmin@, sysadmin@, auditor@, sysmember@) plus projects and roles
// named "[test] …" (removed after the run). sysmember@ is the only account that gets disabled.
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { API, DEMO, ORIGIN, Session, bearer, hex, scoped, user } from './lib.mjs';

const ADMIN = '/api/v1/admin';
const ALL = [
  'system.projects.view',
  'system.projects.manage',
  'system.users.view',
  'system.users.manage',
  'system.team.view',
  'system.team.manage',
  'system.roles.manage',
  'system.audit.view',
];
const AUDITOR = ['system.projects.view', 'system.users.view', 'system.team.view', 'system.audit.view'];
const testName = (label) => `[test] ${label} ${hex(3)}`;

const superAdmin = new Session();
const sysAdmin = new Session();
const auditor = new Session();
const member = new Session();
const plainOwner = new Session(); // demo owner without administrator access
let ids; // user ids by email
let systemRoles; // role ids by built-in key
const created = new Set();

async function createProject(owner_email) {
  const r = await superAdmin.mutate(`${ADMIN}/projects`, { name: testName('Project'), owner_email });
  assert.equal(r.status, 201, JSON.stringify(r.body));
  created.add(r.body.id);
  return r.body;
}

async function audit(s, projectId) {
  const r = await s.json(`${ADMIN}/audit${projectId ? `?project_id=${encodeURIComponent(projectId)}` : ''}`);
  assert.equal(r.status, 200, JSON.stringify(r.body));
  return r.body;
}

before(async () => {
  await superAdmin.login('superadmin@karbird.local');
  await sysAdmin.login('sysadmin@karbird.local');
  await auditor.login('auditor@karbird.local');
  await member.login('sysmember@karbird.local');
  await plainOwner.login('admin@karbird.local');
  const users = await superAdmin.json(`${ADMIN}/users`);
  assert.equal(users.status, 200, JSON.stringify(users.body));
  ids = Object.fromEntries(users.body.map((u) => [u.email, u.id]));
  const roles = await superAdmin.json(`${ADMIN}/roles`);
  systemRoles = Object.fromEntries(roles.body.filter((r) => r.builtin).map((r) => [r.builtin, r.id]));
  assert.deepEqual(Object.keys(systemRoles).sort(), ['admin', 'auditor', 'super_admin']);
});

after(async () => {
  for (const id of created) await superAdmin.mutate(`${ADMIN}/projects/${id}`, {}, 'DELETE');
  await superAdmin.mutate(`${ADMIN}/members/${ids['sysmember@karbird.local']}`, {}, 'DELETE');
  await superAdmin.mutate(`${ADMIN}/users/${ids['sysmember@karbird.local']}/enable`, {});
});

test('/session reports administrator access only to administrators', async () => {
  const sa = (await superAdmin.json('/api/v1/session')).body.system;
  assert.deepEqual([sa.role, sa.role_name, sa.is_super_admin], ['super_admin', 'Super Admin', true]);
  assert.deepEqual(sa.permissions, ALL);
  const ad = (await sysAdmin.json('/api/v1/session')).body.system;
  assert.deepEqual([ad.role, ad.is_super_admin], ['admin', false]);
  const au = (await auditor.json('/api/v1/session')).body.system;
  assert.deepEqual([au.role, au.permissions], ['auditor', AUDITOR]);
  for (const s of [member, plainOwner]) {
    const r = await s.json('/api/v1/session');
    assert.equal(r.status, 200);
    assert.equal(r.body.system ?? null, null, 'no administrator access');
  }
});

test('the admin API needs a session; everyone without administrator access gets 404', async () => {
  const reads = ['/overview', '/permissions', '/projects', '/users', '/members', '/roles', '/audit'];
  for (const p of reads) {
    assert.equal((await fetch(`${API}${ADMIN}${p}`)).status, 401, `anonymous GET ${p}`);
    for (const s of [member, plainOwner]) {
      assert.equal((await s.json(`${ADMIN}${p}`)).status, 404, `GET ${p} without access`);
    }
  }
  const create = await plainOwner.mutate(`${ADMIN}/projects`, { name: testName('Nope') });
  assert.equal(create.status, 404, 'a project owner is not an administrator');
  const disable = await member.mutate(`${ADMIN}/users/${ids['admin@karbird.local']}/disable`, {});
  assert.equal(disable.status, 404);
});

test('the catalog lists every administrator permission once', async () => {
  const r = await auditor.json(`${ADMIN}/permissions`);
  assert.equal(r.status, 200);
  assert.deepEqual(r.body.map((p) => p.id), ALL);
  assert.ok(r.body.every((p) => p.label && p.group));
});

test('an Auditor reads everything and changes nothing', async () => {
  for (const p of ['/overview', '/projects', '/users', '/members', '/roles', '/audit']) {
    assert.equal((await auditor.json(`${ADMIN}${p}`)).status, 200, `GET ${p}`);
  }
  const overview = (await auditor.json(`${ADMIN}/overview`)).body;
  assert.ok(overview.projects >= 2 && overview.users >= 4 && overview.administrators >= 3, JSON.stringify(overview));
  const target = ids['sysmember@karbird.local'];
  const attempts = [
    [`${ADMIN}/projects`, { name: testName('Auditor') }, 'POST'],
    [`${ADMIN}/projects/demo`, { name: 'Renamed' }, 'PATCH'],
    [`${ADMIN}/projects/demo`, {}, 'DELETE'],
    [`${ADMIN}/projects/demo/environments`, { name: 'x' }, 'POST'],
    [`${ADMIN}/projects/demo/members`, { email: 'sysmember@karbird.local', role: 'viewer' }, 'PUT'],
    [`${ADMIN}/users/${target}/disable`, {}, 'POST'],
    [`${ADMIN}/users/${target}/sessions/revoke`, {}, 'POST'],
    [`${ADMIN}/members`, { email: 'sysmember@karbird.local', role_id: systemRoles.auditor }, 'POST'],
    [`${ADMIN}/roles`, { name: testName('Role'), permissions: [] }, 'POST'],
  ];
  for (const [path, body, method] of attempts) {
    const r = await auditor.mutate(path, body, method);
    assert.equal(r.status, 403, `${method} ${path}: ${JSON.stringify(r.body)}`);
  }
});

test('projects: create with an owner, rename, add an environment, manage members, delete', async () => {
  const p = await createProject('sysmember@karbird.local');
  assert.deepEqual(p.owners, ['sysmember@karbird.local']);
  assert.deepEqual(p.environments.map((e) => e.name), ['production']);
  assert.equal(p.protected, false);
  let mine = (await member.json('/api/v1/session')).body.projects.find((x) => x.id === p.id);
  assert.equal(mine?.role, 'owner', 'the named owner sees the project at once');

  const newName = testName('Renamed');
  const renamed = await sysAdmin.mutate(`${ADMIN}/projects/${p.id}`, { name: newName }, 'PATCH');
  assert.deepEqual([renamed.status, renamed.body.name], [200, newName]);

  const env = await sysAdmin.mutate(`${ADMIN}/projects/${p.id}/environments`, { name: 'staging' });
  assert.equal(env.status, 201, JSON.stringify(env.body));
  assert.deepEqual(env.body.environments.map((e) => e.name).sort(), ['production', 'staging']);
  const dup = await sysAdmin.mutate(`${ADMIN}/projects/${p.id}/environments`, { name: 'staging' });
  assert.equal(dup.status, 409);
  const blank = await sysAdmin.mutate(`${ADMIN}/projects/${p.id}/environments`, { name: '   ' });
  assert.equal(blank.status, 400);

  const put = await sysAdmin.mutate(`${ADMIN}/projects/${p.id}/members`, { email: 'auditor@karbird.local', role: 'viewer' }, 'PUT');
  assert.deepEqual([put.status, put.body.role], [200, 'viewer'], JSON.stringify(put.body));
  const promoted = await sysAdmin.mutate(`${ADMIN}/projects/${p.id}/members`, { email: 'auditor@karbird.local', role: 'operator' }, 'PUT');
  assert.equal(promoted.body.role, 'operator');
  const badRole = await sysAdmin.mutate(`${ADMIN}/projects/${p.id}/members`, { email: 'auditor@karbird.local', role: 'root' }, 'PUT');
  assert.equal(badRole.status, 400);
  const unknown = await sysAdmin.mutate(`${ADMIN}/projects/${p.id}/members`, { email: `nobody-${hex(4)}@example.test`, role: 'viewer' }, 'PUT');
  assert.equal(unknown.status, 400);
  const lastOwner = await sysAdmin.mutate(`${ADMIN}/projects/${p.id}/members/${ids['sysmember@karbird.local']}`, {}, 'DELETE');
  assert.equal(lastOwner.status, 409, 'a project keeps at least one owner');
  const members = await auditor.json(`${ADMIN}/projects/${p.id}/members`);
  assert.deepEqual(members.body.map((m) => m.email).sort(), ['auditor@karbird.local', 'sysmember@karbird.local']);
  const removed = await sysAdmin.mutate(`${ADMIN}/projects/${p.id}/members/${ids['auditor@karbird.local']}`, {}, 'DELETE');
  assert.equal(removed.status, 204);

  const del = await sysAdmin.mutate(`${ADMIN}/projects/${p.id}`, {}, 'DELETE');
  assert.equal(del.status, 204);
  created.delete(p.id);
  mine = (await member.json('/api/v1/session')).body.projects.find((x) => x.id === p.id);
  assert.equal(mine, undefined, 'members lose the project');
  assert.equal((await sysAdmin.json(`${ADMIN}/projects/${p.id}/members`)).status, 404);
  assert.equal((await sysAdmin.mutate(`${ADMIN}/projects/${p.id}`, {}, 'DELETE')).status, 404);

  const entries = await audit(auditor, p.id);
  const actions = entries.map((e) => e.action);
  for (const a of ['admin.project.create', 'admin.project.rename', 'admin.environment.create', 'admin.member.put', 'admin.member.remove', 'admin.project.delete']) {
    assert.ok(actions.includes(a), `${a} audited: ${actions}`);
  }
  const deletion = entries.find((e) => e.action === 'admin.project.delete');
  assert.deepEqual([deletion.actor_email, deletion.outcome], ['sysadmin@karbird.local', 'succeeded']);
});

test('the demo project cannot be deleted; unknown or malformed projects are 404', async () => {
  const r = await superAdmin.mutate(`${ADMIN}/projects/demo`, {}, 'DELETE');
  assert.equal(r.status, 409);
  const demo = (await superAdmin.json(`${ADMIN}/projects`)).body.find((p) => p.id === 'demo');
  assert.equal(demo.protected, true);
  for (const id of [`p-${hex(8)}`, 'bad%20id', '%3Cscript%3E']) {
    assert.equal((await superAdmin.mutate(`${ADMIN}/projects/${id}`, { name: 'x' }, 'PATCH')).status, 404, id);
  }
});

test('administrator access does not open project data', async () => {
  for (const s of [superAdmin, sysAdmin, auditor]) {
    assert.equal((await s.json(`${DEMO}/services`)).status, 404, 'telemetry needs a project membership');
    assert.equal((await s.json('/api/v1/projects/demo/members')).status, 404);
  }
});

test('mutations need the CSRF token and an allowed origin', async () => {
  const body = JSON.stringify({ name: testName('Csrf') });
  const noCsrf = await superAdmin.json(`${ADMIN}/projects`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body,
  });
  assert.equal(noCsrf.status, 403);
  const evil = await superAdmin.json(`${ADMIN}/projects`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: 'http://evil.example', 'x-csrf-token': superAdmin.cookies.get('karbird_csrf') },
    body,
  });
  assert.equal(evil.status, 403);
  const names = (await superAdmin.json(`${ADMIN}/projects`)).body.map((p) => p.name);
  assert.ok(!names.some((n) => n.startsWith('[test] Csrf')), 'nothing was created');
});

test('API tokens never reach the admin API, even an administrator\'s', async () => {
  const p = await createProject();
  const env = p.environments[0];
  const tok = await superAdmin.mutate(`${scoped(p.id, env.id)}/tokens`, { kind: 'api', name: `admin-${hex(3)}` });
  assert.equal(tok.status, 200, JSON.stringify(tok.body));
  const secret = tok.body.secret;
  assert.equal((await bearer(secret, `${scoped(p.id, env.id)}/services`)).status, 200, 'the token itself works');
  for (const path of ['/overview', '/projects', '/users']) {
    assert.equal((await bearer(secret, `${ADMIN}${path}`)).status, 403, `GET ${path}`);
  }
  const post = await bearer(secret, `${ADMIN}/projects`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: '{"name":"x"}' });
  assert.equal(post.status, 403);
  const session = await bearer(secret, '/api/v1/session');
  assert.equal(session.body?.system ?? null, null);
});

test('administrators: no escalation, only a Super Admin touches a Super Admin', async () => {
  const sa = ids['superadmin@karbird.local'];
  const target = ids['sysmember@karbird.local'];
  const grantSuper = await sysAdmin.mutate(`${ADMIN}/members`, { email: 'sysmember@karbird.local', role_id: systemRoles.super_admin });
  assert.equal(grantSuper.status, 403, 'an Administrator cannot grant Super Admin');
  const demote = await sysAdmin.mutate(`${ADMIN}/members/${sa}`, { role_id: systemRoles.auditor }, 'PATCH');
  assert.equal(demote.status, 403);
  assert.equal((await sysAdmin.mutate(`${ADMIN}/members/${sa}`, {}, 'DELETE')).status, 403);
  const self = await superAdmin.mutate(`${ADMIN}/members/${sa}`, { role_id: systemRoles.auditor }, 'PATCH');
  assert.equal(self.status, 403, 'nobody changes their own access');

  const add = await sysAdmin.mutate(`${ADMIN}/members`, { email: 'sysmember@karbird.local', role_id: systemRoles.auditor });
  assert.equal(add.status, 201, JSON.stringify(add.body));
  assert.deepEqual([add.body.role, add.body.permissions], ['auditor', AUDITOR]);
  const again = await sysAdmin.mutate(`${ADMIN}/members`, { email: 'sysmember@karbird.local', role_id: systemRoles.auditor });
  assert.equal(again.status, 409);
  assert.equal((await member.json(`${ADMIN}/overview`)).status, 200, 'access applies at once');
  assert.equal((await member.mutate(`${ADMIN}/projects`, { name: testName('Member') })).status, 403);

  const individual = await sysAdmin.mutate(`${ADMIN}/members/${target}`, { permissions: ['system.audit.view'], based_on: systemRoles.auditor }, 'PATCH');
  assert.equal(individual.status, 200, JSON.stringify(individual.body));
  assert.deepEqual([individual.body.role, individual.body.permissions], ['individual', ['system.audit.view']]);
  const bad = await sysAdmin.mutate(`${ADMIN}/members/${target}`, { permissions: ['telemetry.view'] }, 'PATCH');
  assert.equal(bad.status, 400, 'project permissions are not administrator permissions');
  assert.equal((await member.json(`${ADMIN}/projects`)).status, 403);

  assert.equal((await sysAdmin.mutate(`${ADMIN}/members/${target}`, {}, 'DELETE')).status, 204);
  assert.equal((await member.json(`${ADMIN}/overview`)).status, 404, 'access ends at once');

  const actions = (await audit(auditor)).map((e) => `${e.action}:${e.outcome}`);
  for (const a of ['admin.admin.add:denied', 'admin.admin.update:denied', 'admin.admin.add:succeeded', 'admin.admin.remove:succeeded']) {
    assert.ok(actions.includes(a), `${a} audited`);
  }
});

test('administrator roles: templates within your own permissions; built-ins are read-only', async () => {
  const create = await sysAdmin.mutate(`${ADMIN}/roles`, { name: testName('Support'), permissions: ['system.users.manage'] });
  assert.equal(create.status, 200, JSON.stringify(create.body));
  assert.deepEqual(create.body.permissions, ['system.users.view', 'system.users.manage'], 'implied permissions are added');
  const role = create.body;
  const dupName = await sysAdmin.mutate(`${ADMIN}/roles`, { name: role.name, permissions: [] });
  assert.equal(dupName.status, 409);
  const builtinEdit = await superAdmin.mutate(`${ADMIN}/roles/${systemRoles.auditor}`, { name: 'Auditor', permissions: ALL }, 'PATCH');
  assert.equal(builtinEdit.status, 403);
  assert.equal((await superAdmin.mutate(`${ADMIN}/roles/${systemRoles.super_admin}`, {}, 'DELETE')).status, 403);

  const add = await superAdmin.mutate(`${ADMIN}/members`, { email: 'sysmember@karbird.local', role_id: role.id });
  assert.equal(add.status, 201);
  assert.equal((await superAdmin.mutate(`${ADMIN}/roles/${role.id}`, {}, 'DELETE')).status, 409, 'held roles stay');
  const edit = await sysAdmin.mutate(`${ADMIN}/roles/${role.id}`, { name: role.name, permissions: ['system.audit.view'] }, 'PATCH');
  assert.equal(edit.status, 200);
  const access = (await member.json('/api/v1/session')).body.system;
  assert.deepEqual([access.role, access.permissions], ['custom', ['system.audit.view']], 'holders follow the template');
  assert.equal((await superAdmin.mutate(`${ADMIN}/members/${ids['sysmember@karbird.local']}`, {}, 'DELETE')).status, 204);
  assert.equal((await superAdmin.mutate(`${ADMIN}/roles/${role.id}`, {}, 'DELETE')).status, 204);
});

test('accounts: disabling signs out and blocks sign-in; enabling restores it; never your own', async () => {
  const target = ids['sysmember@karbird.local'];
  const self = await superAdmin.mutate(`${ADMIN}/users/${ids['superadmin@karbird.local']}/disable`, {});
  assert.equal(self.status, 403);
  const superTarget = await sysAdmin.mutate(`${ADMIN}/users/${ids['superadmin@karbird.local']}/disable`, {});
  assert.equal(superTarget.status, 403, 'only a Super Admin disables a Super Admin');
  assert.equal((await sysAdmin.mutate(`${ADMIN}/users/${ids['superadmin@karbird.local']}/sessions/revoke`, {})).status, 403);

  const revoke = await sysAdmin.mutate(`${ADMIN}/users/${target}/sessions/revoke`, {});
  assert.equal(revoke.status, 200);
  assert.ok(revoke.body.revoked >= 1);
  assert.equal((await member.json('/api/v1/session')).status, 401, 'signed out');
  await member.login('sysmember@karbird.local');

  const disable = await sysAdmin.mutate(`${ADMIN}/users/${target}/disable`, {});
  assert.deepEqual([disable.status, disable.body.disabled, disable.body.active_sessions], [200, true, 0]);
  assert.equal((await member.json('/api/v1/session')).status, 401);
  const relogin = await new Session().json('/api/v1/session/login', {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify({ email: 'sysmember@karbird.local', password: user('sysmember@karbird.local').password }),
  });
  assert.equal(relogin.status, 401, 'the right password does not sign a disabled account in');
  const listed = (await auditor.json(`${ADMIN}/users`)).body.find((u) => u.id === target);
  assert.equal(listed.disabled, true);

  const enable = await sysAdmin.mutate(`${ADMIN}/users/${target}/enable`, {});
  assert.deepEqual([enable.status, enable.body.disabled], [200, false]);
  await member.login('sysmember@karbird.local');

  const unknown = await sysAdmin.mutate(`${ADMIN}/users/00000000-0000-0000-0000-000000000000/disable`, {});
  assert.equal(unknown.status, 404);
  const actions = (await audit(auditor)).map((e) => e.action);
  for (const a of ['admin.user.disable', 'admin.user.enable', 'admin.user.sessions_revoke']) assert.ok(actions.includes(a), a);
});

test('audit filter accepts only well-formed project ids', async () => {
  assert.equal((await auditor.json(`${ADMIN}/audit?project_id=${encodeURIComponent("x' OR 1=1")}`)).status, 400);
  const demo = await audit(auditor, 'demo');
  assert.ok(demo.every((e) => e.project_id === 'demo'));
});
