// Token management and read-only API tokens against a locally running stack.
// Run: node --test "tests/integration/*.test.mjs"
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { OTLP, ORIGIN, Session, DEMO, ISO, bearer, eventually } from './lib.mjs';

let admin;
let viewer;
before(async () => {
  admin = new Session();
  await admin.login('admin@controlhub.local');
  viewer = new Session();
  await viewer.login('viewer@controlhub.local');
});

const name = (p) => `${p}-${randomUUID().slice(0, 8)}`;

test('only admins manage tokens, with CSRF, and secrets are shown once', async () => {
  assert.equal((await viewer.json(`${DEMO}/tokens`)).status, 403, 'viewer cannot list tokens');
  assert.equal((await viewer.mutate(`${DEMO}/tokens`, { kind: 'api', name: 'x' })).status, 403);

  const noCsrf = await admin.json(`${DEMO}/tokens`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify({ kind: 'api', name: 'x' }),
  });
  assert.equal(noCsrf.status, 403, 'mutation without CSRF token');

  const bad = await admin.mutate(`${DEMO}/tokens`, { kind: 'api', name: 'x', expires_in_days: 365 });
  assert.equal(bad.status, 400, 'API tokens are limited to 90 days');

  const n = name('cli');
  const created = await admin.mutate(`${DEMO}/tokens`, { kind: 'api', name: n, expires_in_days: 7 });
  assert.equal(created.status, 200, JSON.stringify(created.body));
  assert.match(created.body.secret, /^cha_[A-Za-z0-9_-]{43}$/);
  assert.equal(created.body.token.kind, 'api');
  assert.ok(created.body.token.expires_at, 'API tokens always expire');
  assert.ok(created.body.secret.startsWith(created.body.token.prefix));

  const list = await admin.json(`${DEMO}/tokens`);
  assert.equal(list.status, 200);
  const listed = list.body.find((t) => t.id === created.body.token.id);
  assert.ok(listed && listed.active);
  assert.ok(!JSON.stringify(list.body).includes(created.body.secret), 'secret is never listed');

  const audit = await admin.json(`${DEMO}/audit`);
  assert.ok(audit.body.some((a) => a.action === 'token.create' && a.target === created.body.token.id));
});

test('API tokens are read-only, bound to one environment and revocable', async () => {
  const created = await admin.mutate(`${DEMO}/tokens`, { kind: 'api', name: name('mcp') });
  assert.equal(created.status, 200);
  const token = created.body.secret;

  const services = await bearer(token, `${DEMO}/services`);
  assert.equal(services.status, 200);
  assert.ok(services.body.services.length > 0);
  const session = await bearer(token, '/api/v1/session');
  assert.equal(session.status, 200);
  assert.deepEqual(
    session.body.projects.map((p) => [p.id, p.role, p.environments.map((e) => e.id)]),
    [['demo', 'viewer', ['local-demo']]],
  );

  assert.equal((await bearer(token, `${ISO}/services`)).status, 404, 'other tenant');
  assert.equal((await bearer(token, `${DEMO}/tokens`)).status, 403, 'cannot manage tokens');
  const post = await bearer(token, `${DEMO}/tokens`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ kind: 'api', name: 'escalate' }),
  });
  assert.equal(post.status, 403, 'mutations are rejected for API tokens');
  assert.equal((await bearer(token, '/api/v1/session/logout', { method: 'POST' })).status, 403);
  assert.equal((await bearer('cha_invalid', `${DEMO}/services`)).status, 401);
  assert.equal((await bearer('chs_not-an-api-token', `${DEMO}/services`)).status, 401);

  const revoked = await admin.mutate(`${DEMO}/tokens/${created.body.token.id}/revoke`, { reason: 'test' });
  assert.equal(revoked.status, 200);
  assert.equal(revoked.body.active, false);
  assert.equal((await bearer(token, `${DEMO}/services`)).status, 401, 'revoked immediately');
  assert.equal((await admin.mutate(`${DEMO}/tokens/${created.body.token.id}/revoke`, {})).status, 400);

  const audit = await admin.json(`${DEMO}/audit`);
  const entry = audit.body.find((a) => a.action === 'token.revoke' && a.target === created.body.token.id);
  assert.ok(entry);
  assert.equal(entry.reason, 'test');
});

test('ingestion token rotation keeps the old token for the grace period only', async () => {
  const created = await admin.mutate(`${DEMO}/tokens`, { kind: 'ingest', name: name('sdk') });
  assert.equal(created.status, 200);
  assert.match(created.body.secret, /^chi_/);
  const send = (token) =>
    fetch(`${OTLP}/api/v1/events/batch`, {
      method: 'POST',
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      body: JSON.stringify({
        events: [
          {
            event_id: randomUUID(),
            event_type: 'token.test',
            service: { name: 'token-test' },
            event_time: `${BigInt(Date.now()) * 1_000_000n}`,
          },
        ],
      }),
    }).then((r) => r.status);
  assert.equal(await send(created.body.secret), 200);

  const rotated = await admin.mutate(`${DEMO}/tokens/${created.body.token.id}/rotate`, { grace_seconds: 600 });
  assert.equal(rotated.status, 200, JSON.stringify(rotated.body));
  assert.notEqual(rotated.body.secret, created.body.secret);
  assert.equal(rotated.body.token.name, created.body.token.name);
  assert.equal(await send(rotated.body.secret), 200, 'new token works');
  assert.equal(await send(created.body.secret), 200, 'old token works during grace');

  const list = await admin.json(`${DEMO}/tokens`);
  const old = list.body.find((t) => t.id === created.body.token.id);
  assert.ok(old.expires_at, 'old token now expires');
  assert.ok(Date.parse(old.expires_at) - Date.now() <= 600_000);

  const revoked = await admin.mutate(`${DEMO}/tokens/${created.body.token.id}/revoke`, {});
  assert.equal(revoked.status, 200);
  // The ingest service caches credentials for up to 30 s.
  await eventually(async () => assert.equal(await send(created.body.secret), 401), { timeoutMs: 45_000 });
  assert.equal(await send(rotated.body.secret), 200);
});

test('tokens of another tenant are invisible', async () => {
  const iso = new Session();
  await iso.login('isolation@controlhub.local');
  const created = await admin.mutate(`${DEMO}/tokens`, { kind: 'api', name: name('iso') });
  assert.equal((await iso.json(`${DEMO}/tokens`)).status, 404);
  assert.equal((await iso.mutate(`${ISO}/tokens/${created.body.token.id}/revoke`, {})).status, 404);
});
