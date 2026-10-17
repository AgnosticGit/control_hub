// Status codes, limits and headers of every REST route, driven by the committed OpenAPI
// document so new routes are covered automatically.
// Run: node --test "tests/integration/*.test.mjs"
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { API, ORIGIN, root, user, Session, DEMO, ISO, nowNs, postOversized } from './lib.mjs';

const spec = JSON.parse(readFileSync(join(root, 'schemas', 'generated', 'openapi.json'), 'utf8'));
// Sign-up is public like login; its Origin/input/rate rules are covered in signup.test.mjs.
// Invite preview answers anyone holding the link; covered in invites.test.mjs.
const PUBLIC = new Set(['/api/v1/health', '/api/v1/ready', '/api/v1/session/login', '/api/v1/session/register', '/api/v1/invites/preview']);
// OAuth starts without an existing account session and validates its own one-time state.
PUBLIC.add('/api/v1/auth/google/start');
PUBLIC.add('/api/v1/auth/google/callback');
// Enrollment authenticates with its one-time code in the request body.
PUBLIC.add('/api/v1/control/enroll');
// The live route is a WebSocket upgrade; its rules are covered in e2e-flow.test.mjs.
const WEBSOCKET = /\/live$/;
const FAKE_UUID = '00000000-0000-4000-8000-000000000000';
const FAKE_TRACE = 'ab'.repeat(16);

/** Concrete URL for an OpenAPI path template in the given project/environment. */
function concrete(path, project = 'demo', env = 'local-demo') {
  const concretePath = path
    .replace('{project_id}', project)
    .replace('{environment_id}', env)
    .replace('{plan_id}', FAKE_UUID)
    .replace('{command_id}', FAKE_UUID)
    .replace('{id}', FAKE_UUID)
    .replace('{token_id}', FAKE_UUID)
    .replace('{invite_id}', FAKE_UUID)
    .replace('{session_id}', FAKE_UUID)
    .replace('{role_id}', FAKE_UUID)
    .replace('{user_id}', FAKE_UUID)
    .replace('{trace_id}', FAKE_TRACE);
  return path.endsWith('/metrics/overview') ? `${concretePath}?name=orders` : concretePath;
}

const operations = Object.entries(spec.paths).flatMap(([path, item]) =>
  Object.keys(item)
    .filter((m) => ['get', 'post', 'put', 'patch', 'delete'].includes(m))
    .map((method) => ({ path, method: method.toUpperCase() })),
);
const scopedGets = operations.filter((o) => o.method === 'GET' && o.path.includes('{project_id}') && !WEBSOCKET.test(o.path));
const mutations = operations.filter((o) => o.method !== 'GET' && !PUBLIC.has(o.path) && !o.path.startsWith('/api/v1/control/'));

let admin;
let iso;
before(async () => {
  admin = new Session();
  await admin.login('admin@karbird.local');
  iso = new Session();
  await iso.login('isolation@karbird.local');
});

test('the committed OpenAPI document is a valid 3.x document with typed error bodies', () => {
  assert.match(spec.openapi, /^3\./);
  assert.ok(spec.info?.title);
  assert.ok(operations.length >= 25, `only ${operations.length} operations`);
  assert.ok(spec.components?.schemas?.ErrorBody, 'ErrorBody schema');
  for (const { path, method } of operations) {
    const op = spec.paths[path][method.toLowerCase()];
    assert.ok(op.responses && Object.keys(op.responses).length > 0, `${method} ${path} documents responses`);
  }
});

test('health and readiness are public, JSON and never cached', async () => {
  for (const [path, status] of [
    ['/api/v1/health', 'ok'],
    ['/api/v1/ready', 'ready'],
  ]) {
    const res = await fetch(`${API}${path}`);
    assert.equal(res.status, 200, path);
    assert.deepEqual(await res.json(), { status });
    assert.equal(res.headers.get('cache-control'), 'no-store');
    assert.equal(res.headers.get('x-content-type-options'), 'nosniff');
  }
});

test('unknown routes return 404 and wrong methods are refused', async () => {
  assert.equal((await fetch(`${API}/api/v1/does-not-exist`)).status, 404);
  assert.equal((await fetch(`${API}/api/v1/openapi.json`)).status, 404, 'the API does not serve its schema');
  const wrong = await fetch(`${API}/api/v1/session/login`);
  assert.equal(wrong.status, 405);
});

test('OAuth entry points work without a session but never accept a missing callback state',async()=>{
  const start=await fetch(`${API}/api/v1/auth/google/start`,{redirect:'manual'});
  await start.arrayBuffer();
  assert.ok([307,404].includes(start.status));
  assert.ok(!start.headers.getSetCookie().some(cookie=>cookie.startsWith('karbird_session=')));
  const callback=await fetch(`${API}/api/v1/auth/google/callback`,{redirect:'manual'});
  await callback.arrayBuffer();
  assert.ok([401,404].includes(callback.status));
});

test('every non-public route requires authentication (401 with a typed error body)', async () => {
  for (const { path, method } of operations) {
    if (PUBLIC.has(path) || WEBSOCKET.test(path)) continue;
    const res = await fetch(`${API}${concrete(path)}`, {
      method,
      headers: method === 'GET' ? {} : { 'content-type': 'application/json', origin: ORIGIN },
      body: method === 'GET' ? undefined : path.endsWith('/control/commands/{command_id}/complete')
        ? JSON.stringify({ succeeded: true, observed_state: 'running' })
        : path === '/api/v1/control/report'
          ? JSON.stringify({ container_id: 'a'.repeat(64), container_name: 'probe', state: 'running' }) : '{}',
    });
    assert.equal(res.status, 401, `${method} ${path}`);
    const body = await res.json();
    assert.equal(body.code, 'unauthenticated', `${method} ${path}`);
    assert.equal(res.headers.get('cache-control'), 'no-store');
  }
});

test('a forged or expired session cookie is rejected like no session', async () => {
  const res = await fetch(`${API}/api/v1/session`, { headers: { cookie: 'karbird_session=chs_forged' } });
  assert.equal(res.status, 401);
  const bearerSession = await fetch(`${API}/api/v1/session`, { headers: { authorization: 'Bearer chs_forged' } });
  assert.equal(bearerSession.status, 401, 'session secrets are not accepted as bearer tokens');
  const basic = await fetch(`${API}/api/v1/session`, { headers: { authorization: 'Basic YWRtaW46YWRtaW4=' } });
  assert.equal(basic.status, 401, 'no basic auth');
});

test('another tenant gets 404 (not 403) on every scoped read, so existence is not leaked', async () => {
  for (const { path } of scopedGets) {
    const r = await iso.json(concrete(path));
    assert.equal(r.status, 404, `isolation user on demo ${path}: ${r.status}`);
    assert.equal(r.body?.code, 'not_found');
  }
  for (const { path } of scopedGets) {
    const r = await admin.json(concrete(path, 'isolation', 'isolation-test'));
    assert.equal(r.status, 404, `demo admin on isolation ${path}: ${r.status}`);
  }
});

test('unknown projects/environments and malformed IDs are 404 for members too', async () => {
  for (const [p, e] of [
    ['demo', 'nope'],
    ['nope', 'local-demo'],
    ['demo', 'isolation-test'],
    ['..', 'local-demo'],
    ['demo', '%3Cscript%3E'],
  ]) {
    const r = await admin.json(`/api/v1/projects/${p}/environments/${e}/services`);
    assert.equal(r.status, 404, `${p}/${e}`);
  }
  assert.equal((await admin.json(`${DEMO}/actions/not-a-uuid`)).status, 404);
  assert.equal((await admin.json(`${DEMO}/actions/${FAKE_UUID}`)).status, 404);
});

test('every cookie-authenticated mutation requires the CSRF header and an allowed Origin', async () => {
  const csrf = admin.cookies.get('karbird_csrf');
  assert.ok(csrf);
  const variants = [
    ['no CSRF header', { origin: ORIGIN }],
    ['wrong CSRF header', { origin: ORIGIN, 'x-csrf-token': 'chc_wrong' }],
    ['foreign Origin', { origin: 'https://evil.example', 'x-csrf-token': csrf }],
    ['null Origin', { origin: 'null', 'x-csrf-token': csrf }],
  ];
  for (const { path, method } of mutations) {
    for (const [name, headers] of variants) {
      const r = await admin.json(concrete(path), {
        method,
        headers: { 'content-type': 'application/json', ...headers },
        body: JSON.stringify({ kind: 'api', name: 'csrf-probe', reason: 'csrf probe', comment: 'csrf probe' }),
      });
      assert.equal(r.status, 403, `${method} ${path} with ${name}: ${r.status}`);
      assert.equal(r.body?.code, 'forbidden');
    }
  }
  // The session is still usable afterwards (none of the attempts logged it out).
  assert.equal((await admin.json('/api/v1/session')).status, 200);
});

test('CSRF token of one session does not work for another session', async () => {
  const other = new Session();
  await other.login('admin@karbird.local');
  const r = await admin.json(`${DEMO}/tokens`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN, 'x-csrf-token': other.cookies.get('karbird_csrf') },
    body: JSON.stringify({ kind: 'api', name: 'cross-session' }),
  });
  assert.equal(r.status, 403);
});

test('session cookies are HttpOnly (session) / readable (CSRF), SameSite=Strict and scoped to /', async () => {
  const u = user('viewer@karbird.local');
  const res = await fetch(`${API}/api/v1/session/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: ORIGIN },
    body: JSON.stringify({ email: u.email, password: u.password }),
  });
  assert.equal(res.status, 200);
  const cookies = res.headers.getSetCookie();
  const session = cookies.find((c) => c.startsWith('karbird_session='));
  const csrf = cookies.find((c) => c.startsWith('karbird_csrf='));
  assert.ok(session && csrf, JSON.stringify(cookies.map((c) => c.split('=')[0])));
  assert.match(session, /HttpOnly/i);
  assert.match(session, /SameSite=Strict/i);
  assert.match(session, /Path=\//);
  assert.doesNotMatch(csrf, /HttpOnly/i, 'double-submit CSRF cookie must be readable by the app');
  assert.match(csrf, /SameSite=Strict/i);
  const body = await res.json();
  assert.ok(!JSON.stringify(body).match(/chs_|chc_|password/i), 'login response carries no secrets');
});

test('login validates the body: wrong content type 415, wrong shape 422, oversized fields 401', async () => {
  const text = await fetch(`${API}/api/v1/session/login`, { method: 'POST', headers: { 'content-type': 'text/plain' }, body: 'x' });
  assert.equal(text.status, 415);
  const shape = await fetch(`${API}/api/v1/session/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ email: 1 }),
  });
  assert.equal(shape.status, 422);
  const huge = await fetch(`${API}/api/v1/session/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ email: `${'a'.repeat(300)}@x.local`, password: 'x' }),
  });
  assert.equal(huge.status, 401);
});

test('repeated login failures for one account are throttled (429) without affecting others', async () => {
  const email = `throttle-${randomUUID()}@karbird.local`;
  const attempt = () =>
    fetch(`${API}/api/v1/session/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ email, password: 'wrong' }),
    }).then((r) => r.status);
  for (let i = 0; i < 10; i++) assert.equal(await attempt(), 401, `attempt ${i + 1}`);
  assert.equal(await attempt(), 429);
  // Case and whitespace variants hit the same bucket.
  const variant = await fetch(`${API}/api/v1/session/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ email: `  ${email.toUpperCase()} `, password: 'wrong' }),
  });
  assert.equal(variant.status, 429);
  const other = new Session();
  await other.login('viewer@karbird.local');
});

test('request bodies over 1 MiB are refused with 413', async () => {
  assert.equal(await postOversized(`${API}/api/v1/session/login`, { 'content-type': 'application/json' }, 2 * 1024 * 1024), 413);
  assert.equal(
    await postOversized(
      `${API}${DEMO}/tokens`,
      { 'content-type': 'application/json', origin: ORIGIN, cookie: admin.cookieHeader(), 'x-csrf-token': admin.cookies.get('karbird_csrf') },
      2 * 1024 * 1024,
    ),
    413,
  );
});

test('/session and /projects list only the principal’s own memberships', async () => {
  const a = await admin.json('/api/v1/projects');
  assert.equal(a.status, 200);
  assert.deepEqual(a.body.map((p) => p.id), ['demo']);
  const env = a.body[0].environments.find((e) => e.id === 'local-demo');
  assert.equal(env.is_demo, true, 'demo environments are flagged');
  const i = await iso.json('/api/v1/projects');
  assert.deepEqual(i.body.map((p) => p.id), ['isolation']);
  const envs = Object.fromEntries(i.body[0].environments.map((e) => [e.id, e.is_demo]));
  assert.deepEqual(envs, { 'isolation-prod': false, 'isolation-test': true });
  const s = await admin.json('/api/v1/session');
  assert.equal(s.body.user.email, 'admin@karbird.local');
});

const RANGE_ROUTES = ['services', 'topology', 'traces', 'logs', 'events', 'timeline', 'coverage'];

test('time ranges are validated on every telemetry route (400)', async () => {
  const now = nowNs();
  const day = 24n * 3600n * 1_000_000_000n;
  const cases = [
    ['from after to', `from=${now}&to=${now - 1_000_000_000n}`, /before/],
    ['empty range', `from=${now}&to=${now}`, /before/],
    ['over 7 days', `from=${now - 8n * day}&to=${now}`, /7 days/],
    ['not a number', 'from=yesterday', /nanosecond/],
    ['float', `to=${now}.5`, /nanosecond/],
    ['overflow', 'to=99999999999999999999', /nanosecond/],
  ];
  for (const route of RANGE_ROUTES) {
    // Log reads that may use the archive allow 31 days (ADR-0015); ClickHouse-only reads keep 7.
    const logs = route === 'logs';
    for (const [name, qs, msg] of cases) {
      const r = await admin.json(`${DEMO}/${route}?${qs}${logs && name === 'over 7 days' ? '&source=hot' : ''}`);
      assert.equal(r.status, 400, `${route} ${name}: ${r.status}`);
      assert.equal(r.body.code, 'bad_request');
      assert.match(r.body.message, msg, `${route} ${name}`);
    }
    const ok = await admin.json(`${DEMO}/${route}?from=${now - 7n * day}&to=${now}`);
    assert.equal(ok.status, 200, `${route} exactly 7 days is allowed`);
  }
  const longLogs = await admin.json(`${DEMO}/logs?from=${now - 31n * day}&to=${now}`);
  assert.equal(longLogs.status, 200, 'logs: 31 days may be read through the archive');
  const tooLong = await admin.json(`${DEMO}/logs?from=${now - 31n * day - 1n}&to=${now}`);
  assert.equal(tooLong.status, 400);
  assert.match(tooLong.body.message, /31 days/);
});

test('filters are validated: trace IDs, cursors and search text length', async () => {
  const bad = [
    [`${DEMO}/logs?trace_id=xyz`, /trace id/],
    [`${DEMO}/logs?trace_id=${'A'.repeat(32)}0`, /trace id/],
    [`${DEMO}/logs?cursor=not-base64!!`, /cursor/],
    [`${DEMO}/logs?cursor=${Buffer.from('{"t":"x","id":"y"}').toString('base64url')}`, /cursor/],
    [`${DEMO}/traces?cursor=${Buffer.from('[]').toString('base64url')}`, /cursor/],
    [`${DEMO}/traces?cursor=${Buffer.from(JSON.stringify({ t: '1', id: 'x'.repeat(300) })).toString('base64url')}`, /cursor/],
    [`${DEMO}/logs?q=${'x'.repeat(257)}`, /too long/],
    [`${DEMO}/traces/not-a-trace`, /trace id/],
    [`${DEMO}/traces/${'ab'.repeat(17)}`, /trace id/],
  ];
  for (const [path, msg] of bad) {
    const r = await admin.json(path);
    assert.equal(r.status, 400, `${path}: ${r.status} ${JSON.stringify(r.body)}`);
    assert.match(r.body.message, msg, path);
  }
  const typed = await admin.request(`${DEMO}/traces?limit=abc`);
  assert.equal(typed.status, 400, 'non-numeric limit');
  const unknownTrace = await admin.json(`${DEMO}/traces/${FAKE_TRACE}`);
  assert.equal(unknownTrace.status, 404);
  const q = await admin.json(`${DEMO}/logs?q=${encodeURIComponent("' OR 1=1 --%_\\")}`);
  assert.equal(q.status, 200, 'search text is data, not query syntax');
  assert.equal(q.body.logs.length, 0);
});

test('limits are clamped server-side', async () => {
  const logs = await admin.json(`${DEMO}/logs?limit=100000`);
  assert.equal(logs.status, 200);
  assert.ok(logs.body.logs.length <= 1000, `logs: ${logs.body.logs.length}`);
  const one = await admin.json(`${DEMO}/logs?limit=0`);
  assert.ok(one.body.logs.length <= 1, 'limit=0 is clamped to 1');
  const traces = await admin.json(`${DEMO}/traces?limit=100000`);
  assert.ok(traces.body.traces.length <= 500, `traces: ${traces.body.traces.length}`);
  const events = await admin.json(`${DEMO}/events?limit=100000`);
  assert.ok(events.body.events.length <= 1000);
  const timeline = await admin.json(`${DEMO}/timeline?limit=1`);
  assert.equal(timeline.status, 200);
  assert.ok(timeline.body.items.filter((i) => i.type === 'span').length <= 1);
  assert.ok(timeline.body.items.filter((i) => i.type === 'log').length <= 1);
  assert.equal(timeline.body.spans_truncated, true, 'demo has more than one span in 15 min');
  assert.equal(timeline.body.completeness.truncated, true);
});

test('logs and traces paginate with opaque cursors, newest first, without overlap', async () => {
  for (const [route, key, idKey, timeKey] of [
    ['logs', 'logs', 'id', 'time_ns'],
    ['traces', 'traces', 'trace_id', 'start_ns'],
  ]) {
    const first = await admin.json(`${DEMO}/${route}?limit=5`);
    assert.equal(first.status, 200);
    assert.equal(first.body[key].length, 5, `${route} page size`);
    assert.ok(first.body.next_cursor, `${route} has more`);
    assert.equal(first.body.completeness.truncated, true);
    const times = first.body[key].map((x) => BigInt(x[timeKey]));
    for (let i = 1; i < times.length; i++) assert.ok(times[i - 1] >= times[i], `${route} newest first`);
    const second = await admin.json(`${DEMO}/${route}?limit=5&cursor=${first.body.next_cursor}`);
    assert.equal(second.status, 200);
    const ids = new Set(first.body[key].map((x) => x[idKey]));
    assert.ok(second.body[key].every((x) => !ids.has(x[idKey])), `${route} pages overlap`);
    assert.ok(BigInt(second.body[key][0][timeKey]) <= times[times.length - 1], `${route} second page is older`);
  }
});

test('log filters: min_severity, service and text search are applied server-side', async () => {
  const errors = await admin.json(`${DEMO}/logs?min_severity=17&limit=200`);
  assert.equal(errors.status, 200);
  assert.ok(errors.body.logs.every((l) => l.severity_number >= 17));
  const services = await admin.json(`${DEMO}/services`);
  const orders = services.body.services.find((s) => s.name === 'orders');
  const byService = await admin.json(`${DEMO}/logs?service_id=${orders.service_id}&limit=100`);
  assert.ok(byService.body.logs.length > 0);
  assert.ok(byService.body.logs.every((l) => l.service_name === 'orders'));
  const text = await admin.json(`${DEMO}/logs?q=ORDER&limit=100`);
  assert.ok(text.body.logs.length > 0, 'case-insensitive search finds demo logs');
  assert.ok(text.body.logs.every((l) => /order/i.test(l.body)));
  const errTraces = await admin.json(`${DEMO}/traces?only_errors=true&limit=100`);
  assert.ok(errTraces.body.traces.every((t) => t.error_count > 0));
});

test('coverage lists explicit gaps and audit is readable by viewers but scoped', async () => {
  const c = await admin.json(`${DEMO}/coverage`);
  assert.equal(c.status, 200);
  assert.ok(c.body.unsupported.some((u) => /metrics/i.test(u)));
  assert.ok(c.body.services.length > 0);
  const viewer = new Session();
  await viewer.login('viewer@karbird.local');
  const audit = await viewer.json(`${DEMO}/audit`);
  assert.equal(audit.status, 200);
  assert.ok(Array.isArray(audit.body) && audit.body.length <= 200);
  for (const a of audit.body) {
    assert.ok(!/chs_|chc_|cha_[A-Za-z0-9_-]{20,}|chi_[A-Za-z0-9_-]{20,}/.test(JSON.stringify(a)), 'audit never contains secrets');
  }
  const isoAudit = await iso.json(`${ISO}/audit`);
  assert.equal(isoAudit.status, 200);
  const demoIds = new Set(audit.body.map((a) => a.id));
  assert.ok(isoAudit.body.every((a) => !demoIds.has(a.id)), 'audit entries do not cross tenants');
});

test('token management validates names, kinds, expiry, grace and unknown IDs', async () => {
  const bad = [
    [{ kind: 'api', name: '' }, 400],
    [{ kind: 'api', name: '<script>alert(1)</script>' }, 400],
    [{ kind: 'api', name: 'x'.repeat(65) }, 400],
    [{ kind: 'api', name: 'ok', expires_in_days: 0 }, 400],
    [{ kind: 'ingest', name: 'ok', expires_in_days: 366 }, 400],
    [{ kind: 'root', name: 'ok' }, 422],
    [{ name: 'ok' }, 422],
  ];
  for (const [body, status] of bad) {
    const r = await admin.mutate(`${DEMO}/tokens`, body);
    assert.equal(r.status, status, `${JSON.stringify(body)} -> ${r.status}`);
  }
  assert.equal((await admin.mutate(`${DEMO}/tokens/${FAKE_UUID}/revoke`, {})).status, 404);
  assert.equal((await admin.mutate(`${DEMO}/tokens/not-a-uuid/rotate`, {})).status, 404);
  const created = await admin.mutate(`${DEMO}/tokens`, { kind: 'ingest', name: `grace-${randomUUID().slice(0, 8)}`, expires_in_days: 1 });
  assert.equal(created.status, 200);
  const id = created.body.token.id;
  assert.equal((await admin.mutate(`${DEMO}/tokens/${id}/rotate`, { grace_seconds: 86_401 })).status, 400, 'grace over 24 h');
  assert.equal((await admin.mutate(`${DEMO}/tokens/${id}/revoke`, { reason: 'r'.repeat(501) })).status, 400, 'reason over 500 chars');
  assert.equal((await admin.mutate(`${DEMO}/tokens/${id}/revoke`, { reason: 'cleanup' })).status, 200);
  assert.equal((await admin.mutate(`${DEMO}/tokens/${id}/rotate`, {})).status, 400, 'revoked tokens cannot be rotated');
});

test('operators see tokens read-only, viewers not at all; viewers cannot decide actions', async () => {
  const op = new Session();
  await op.login('operator@karbird.local');
  assert.equal((await op.json(`${DEMO}/tokens`)).status, 200, 'Operator has tokens.view');
  assert.equal((await op.mutate(`${DEMO}/tokens`, { kind: 'api', name: 'op' })).status, 403, 'but not tokens.manage');
  const viewer = new Session();
  await viewer.login('viewer@karbird.local');
  assert.equal((await viewer.json(`${DEMO}/tokens`)).status, 403);
  for (const verb of ['approve', 'reject', 'cancel']) {
    const r = await viewer.mutate(`${DEMO}/actions/${FAKE_UUID}/${verb}`, { comment: 'nope nope' });
    assert.equal(r.status, 403, `viewer ${verb}`);
  }
  assert.equal((await op.mutate(`${DEMO}/actions/${FAKE_UUID}/approve`, {})).status, 403, 'operators cannot approve');
  assert.equal((await admin.mutate(`${DEMO}/actions/${FAKE_UUID}/approve`, {})).status, 404);
  assert.equal((await admin.mutate(`${DEMO}/actions/${FAKE_UUID}/reject`, { comment: 'no such plan' })).status, 404);
});

test('logout revokes the session server-side: the old cookie stops working everywhere', async () => {
  const s = new Session();
  await s.login('viewer@karbird.local');
  const stolen = s.cookieHeader();
  const out = await s.request('/api/v1/session/logout', {
    method: 'POST',
    headers: { origin: ORIGIN, 'x-csrf-token': s.cookies.get('karbird_csrf') },
  });
  assert.equal(out.status, 204);
  const cleared = out.headers.getSetCookie();
  assert.ok(cleared.some((c) => /^karbird_session=;/.test(c) && /Max-Age=0/.test(c)), 'session cookie cleared');
  for (const path of ['/api/v1/session', '/api/v1/projects', `${DEMO}/services`]) {
    const r = await fetch(`${API}${path}`, { headers: { cookie: stolen } });
    assert.equal(r.status, 401, `replayed cookie on ${path}`);
  }
});
