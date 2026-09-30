// The developer's own account (.controlhub/owner.env) is the only persistent account: it must
// be able to sign in, directly and through the web app, from both local origins.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { API, WEB, Session, creds, ownerAccount, root } from './lib.mjs';

const owner = ownerAccount();
const skip = owner ? false : 'no .controlhub/owner.env on this machine (bootstrap requires it)';

async function login(base, origin, password = owner.password) {
  const res = await fetch(`${base}/api/v1/session/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin },
    body: JSON.stringify({ email: owner.email, password }),
  });
  return { status: res.status, cookies: res.headers.getSetCookie(), body: await res.json().catch(() => null) };
}

test('the owner signs in and owns the demo project', { skip }, async () => {
  const r = await login(API, 'http://localhost:3000');
  assert.equal(r.status, 200);
  assert.equal(r.body.user.email, owner.email);
  const demo = r.body.projects.find((p) => p.id === 'demo');
  assert.equal(demo?.role, 'owner', `projects: ${JSON.stringify(r.body.projects.map((p) => [p.id, p.role]))}`);
  assert.ok(r.cookies.some((c) => c.startsWith('controlhub_session=') && /HttpOnly/i.test(c)));

  const s = new Session();
  const signedIn = await s.json('/api/v1/session/login', {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: 'http://localhost:3000' },
    body: JSON.stringify({ email: owner.email, password: owner.password }),
  });
  assert.equal(signedIn.status, 200);
  const me = await s.json('/api/v1/session');
  assert.equal(me.status, 200);
  assert.equal(me.body.user.email, owner.email);
});

test('the owner signs in through the web app at 127.0.0.1:3000 and localhost:3000', { skip }, async () => {
  for (const origin of ['http://127.0.0.1:3000', 'http://localhost:3000']) {
    const r = await login(WEB, origin);
    assert.equal(r.status, 200, `origin ${origin}`);
    assert.equal(r.body.user.email, owner.email);
  }
  // Mixed-case input, as typed into the form, is the same account.
  const res = await fetch(`${API}/api/v1/session/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: 'http://127.0.0.1:3000' },
    body: JSON.stringify({ email: ` ${owner.email.toUpperCase()} `, password: owner.password }),
  });
  assert.equal(res.status, 200);
});

test('a wrong owner password is 401 without a session, and the right one still works', { skip }, async () => {
  const bad = await login(API, 'http://127.0.0.1:3000', `${owner.password}-wrong`);
  assert.equal(bad.status, 401);
  assert.ok(!bad.cookies.some((c) => c.startsWith('controlhub_session=')));
  assert.ok(!JSON.stringify(bad.body).includes(owner.password), 'the error never echoes the password');
  // A success resets the failure counter, so this test never locks the owner out.
  assert.equal((await login(API, 'http://127.0.0.1:3000')).status, 200);
});

test('local credentials hold no user accounts and never the owner password', { skip }, () => {
  const text = readFileSync(join(root, '.controlhub', 'local-credentials.json'), 'utf8');
  const local = JSON.parse(text);
  assert.equal(local.users, undefined, 'test accounts must not be part of the persistent credentials');
  assert.equal(local.owner?.email, owner.email);
  assert.ok(!text.includes(owner.password));
  assert.deepEqual(Object.keys(local.ingest_tokens), ['demo-local']);
});

test('test accounts come from the per-run fixture, never from the owner domain', () => {
  const emails = creds.users.map((u) => u.email);
  assert.ok(emails.length > 0);
  for (const e of emails) assert.match(e, /@(controlhub\.local|example\.test)$/);
  if (owner) assert.ok(!emails.includes(owner.email));
});
