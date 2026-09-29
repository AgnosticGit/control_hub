// Self-service sign-up, the new tenant's isolation, and sending telemetry through the web
// app's /otlp proxy with a token created by the new account. Needs `just dev` (sign-up is
// open in dev) and `just demo`.
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { API, DEMO, ISO, ORIGIN, Session, WEB, encodeTraces, eventually, hex, nowNs, scoped } from './lib.mjs';

const PASSWORD = `correct horse ${randomUUID().slice(0, 8)}`;
const email = () => `signup-${randomUUID()}@example.test`;

async function register(body, headers = { origin: ORIGIN }) {
  const res = await fetch(`${API}/api/v1/session/register`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', ...headers },
    body: JSON.stringify(body),
  });
  return { status: res.status, cookies: res.headers.getSetCookie(), body: await res.json().catch(() => null) };
}

let account; // { email, session, scope, project, env }
before(async () => {
  const address = email();
  const session = new Session();
  const r = await session.json('/api/v1/session/register', {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify({ email: `  ${address.toUpperCase()} `, password: PASSWORD, project_name: 'Shop backend' }),
  });
  assert.equal(r.status, 201, JSON.stringify(r.body));
  const project = r.body.projects[0];
  const env = project.environments[0];
  account = { email: address, session, body: r.body, project, env, scope: scoped(project.id, env.id) };
});

test('sign-up creates an owner of a fresh, non-demo project and signs in', async () => {
  const { body, project, env } = account;
  assert.equal(body.user.email, account.email, 'email is normalised');
  assert.equal(body.user.display_name, account.email.split('@')[0]);
  assert.equal(body.projects.length, 1, 'sees only its own project');
  assert.equal(project.name, 'Shop backend');
  assert.equal(project.role, 'owner');
  assert.match(project.id, /^p-[0-9a-f]{32}$/);
  assert.match(project.tenant_id, /^t-[0-9a-f]{32}$/);
  assert.deepEqual([env.name, env.is_demo], ['production', false]);
  assert.match(env.id, /^e-[0-9a-f]{32}$/);

  const me = await account.session.json('/api/v1/session');
  assert.equal(me.status, 200);
  assert.equal(me.body.user.email, account.email);

  // The password works for a normal login afterwards.
  const login = await fetch(`${API}/api/v1/session/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify({ email: account.email, password: PASSWORD }),
  });
  assert.equal(login.status, 200);
});

test('sign-up sets the same hardened cookies as login', async () => {
  const r = await register({ email: email(), password: PASSWORD });
  assert.equal(r.status, 201);
  const session = r.cookies.find((c) => c.startsWith('controlhub_session='));
  const csrf = r.cookies.find((c) => c.startsWith('controlhub_csrf='));
  assert.ok(session && csrf, JSON.stringify(r.cookies));
  assert.match(session, /HttpOnly/);
  assert.match(session, /SameSite=Strict/);
  assert.doesNotMatch(csrf, /HttpOnly/, 'double-submit token is readable by the page');
  assert.ok(!JSON.stringify(r.body).includes(PASSWORD), 'password is never echoed');
  assert.ok(!JSON.stringify(r.body).includes('argon2'), 'hash is never returned');
});

test('a taken email is 409 whatever its case, and does not sign in or change the account', async () => {
  const r = await register({ email: account.email.toUpperCase(), password: 'another long password' });
  assert.equal(r.status, 409);
  assert.equal(r.body.code, 'conflict');
  assert.equal(r.cookies.length, 0);
  const login = await fetch(`${API}/api/v1/session/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify({ email: account.email, password: PASSWORD }),
  });
  assert.equal(login.status, 200, 'original password still works');
});

test('sign-up refuses foreign origins and invalid input without creating anything', async () => {
  const address = email();
  for (const origin of ['https://evil.example', 'null', 'http://localhost:3000.evil.example']) {
    const evil = await register({ email: address, password: PASSWORD }, { origin });
    assert.equal(evil.status, 403, origin);
    assert.equal(evil.body.code, 'forbidden');
    assert.equal(evil.cookies.length, 0);
  }
  for (const body of [
    { email: address, password: 'too short' },
    { email: 'not-an-email', password: PASSWORD },
    { email: address, password: address },
    { email: address, password: PASSWORD, project_name: 'x'.repeat(101) },
  ]) {
    const r = await register(body);
    assert.equal(r.status, 400, JSON.stringify(body));
    assert.equal(r.cookies.length, 0);
  }
  // None of the refused attempts reserved the address.
  const ok = await register({ email: address, password: PASSWORD });
  assert.equal(ok.status, 201);
});

test('a new tenant cannot see the demo or other tenants, and they cannot see it', async () => {
  const { session, scope } = account;
  for (const other of [DEMO, ISO]) {
    assert.equal((await session.json(`${other}/services`)).status, 404, other);
    assert.equal((await session.json(`${other}/tokens`)).status, 404, other);
  }
  const admin = new Session();
  await admin.login('admin@controlhub.local');
  assert.equal((await admin.json(`${scope}/services`)).status, 404, 'demo owner cannot read the new tenant');
  assert.equal((await admin.mutate(`${scope}/tokens`, { kind: 'ingest', name: 'intrude' })).status, 404);
  const projects = await admin.json('/api/v1/projects');
  assert.ok(!projects.body.some((p) => p.id === account.project.id));
});

test('the new owner needs CSRF to create tokens, and telemetry sent via /otlp reaches only its environment', async () => {
  const { session, scope } = account;
  const noCsrf = await session.json(`${scope}/tokens`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify({ kind: 'ingest', name: 'no-csrf' }),
  });
  assert.equal(noCsrf.status, 403);

  const created = await session.mutate(`${scope}/tokens`, { kind: 'ingest', name: 'integration backend' });
  assert.equal(created.status, 200, JSON.stringify(created.body));
  const token = created.body.secret;
  assert.match(token, /^chi_/);

  const service = `signup-svc-${hex(4)}`;
  const start = nowNs();
  const body = encodeTraces(service, [
    { traceId: hex(16), spanId: hex(8), name: 'GET /orders', startNs: start, endNs: start + 5_000_000n },
  ]);
  const send = (t, contentType = 'application/x-protobuf') =>
    fetch(`${WEB}/otlp/v1/traces`, {
      method: 'POST',
      headers: { authorization: `Bearer ${t}`, 'content-type': contentType },
      body,
    });
  assert.equal((await send('chi_wrong')).status, 401, 'ingest checks the token behind the proxy');
  assert.equal((await send(token, 'application/json')).status, 415);
  assert.equal((await send(token)).status, 200);

  await eventually(async () => {
    const r = await session.json(`${scope}/services`);
    assert.equal(r.status, 200);
    assert.ok(r.body.services.some((s) => s.name === service), JSON.stringify(r.body.services.map((s) => s.name)));
  });
  const admin = new Session();
  await admin.login('admin@controlhub.local');
  const demo = await admin.json(`${DEMO}/services`);
  assert.ok(!demo.body.services.some((s) => s.name === service), 'not visible in the demo tenant');
});

test('the web proxy forwards only OTLP paths to ingest', async () => {
  const health = await fetch(`${WEB}/otlp/health`);
  assert.equal(health.status, 404, 'ingest health/admin endpoints are not exposed through the site');
  const metrics = await fetch(`${WEB}/otlp/v1/metrics`, {
    method: 'POST',
    headers: { authorization: 'Bearer chi_wrong', 'content-type': 'application/x-protobuf' },
    body: Buffer.alloc(0),
  });
  assert.equal(metrics.status, 501, 'reaches ingest, which refuses metrics explicitly');
});
