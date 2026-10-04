// Runs a real instrumented process against a local capture server and inspects what leaves it.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { fileURLToPath, pathToFileURL } from 'node:url';

const fixture = fileURLToPath(new URL('./fixtures/instrumented-app.mjs', import.meta.url));
const register = pathToFileURL(fileURLToPath(new URL('../src/register.js', import.meta.url))).href;

const SECRETS = [
  'user-secret-AUTH',
  'cookie-secret-COOKIE',
  'apikey-secret-KEY',
  'body-secret-PW',
  '4111111111111111',
  'resp-cookie-secret',
  'argv-secret-ARG',
  'ingest-token-XYZ',
];

async function capture() {
  const received = [];
  const server = createServer((req, res) => {
    const chunks = [];
    req.on('data', (c) => chunks.push(c));
    req.on('end', () => {
      received.push({ path: req.url, headers: req.headers, body: Buffer.concat(chunks) });
      if (req.url === '/api/v1/events/batch') {
        res.writeHead(202, { 'content-type': 'application/json' });
        res.end(JSON.stringify({ accepted: 1, rejected: [] }));
      } else {
        res.writeHead(200, { 'content-type': 'application/x-protobuf' });
        res.end();
      }
    });
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  return { server, received, url: `http://127.0.0.1:${server.address().port}` };
}

function runFixture(endpoint) {
  const env = {};
  for (const [k, v] of Object.entries(process.env)) {
    if (!k.startsWith('OTEL_') && !k.startsWith('KARBIRD_')) env[k] = v;
  }
  Object.assign(env, {
    KARBIRD_SERVICE_NAME: 'fixture-svc',
    KARBIRD_INGEST_TOKEN: 'ingest-token-XYZ',
    KARBIRD_OTLP_ENDPOINT: `${endpoint}/`,
    KARBIRD_ENVIRONMENT: 'unit-test',
    KARBIRD_HOST_NAME: 'fixture-host',
    KARBIRD_CONTAINER_ID: 'c0ffee'.repeat(10) + 'beef',
  });
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, ['--import', register, fixture, '--secret-arg=argv-secret-ARG'], { env });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (d) => (stdout += d));
    child.stderr.on('data', (d) => (stderr += d));
    const timer = setTimeout(() => child.kill(), 30_000);
    child.on('error', reject);
    child.on('exit', (code) => {
      clearTimeout(timer);
      if (code !== 0) reject(new Error(`fixture exited with ${code}: ${stderr}`));
      else resolve({ stdout, stderr });
    });
  });
}

const cap = await capture();
let result;
try {
  result = await runFixture(cap.url);
} finally {
  cap.server.close();
}
const byPath = (p) => cap.received.filter((r) => r.path === p);

test('exports traces and logs over OTLP/HTTP protobuf with the ingest token', () => {
  for (const path of ['/v1/traces', '/v1/logs']) {
    const reqs = byPath(path);
    assert.ok(reqs.length > 0, `no ${path} export`);
    for (const r of reqs) {
      assert.equal(r.headers.authorization, 'Bearer ingest-token-XYZ');
      assert.match(r.headers['content-type'] ?? '', /application\/x-protobuf/);
    }
  }
  const traces = Buffer.concat(byPath('/v1/traces').map((r) => r.body)).toString('latin1');
  assert.ok(traces.includes('fixture-svc'), 'service.name missing');
  assert.ok(traces.includes('/orders'), 'server span missing');
  assert.ok(traces.includes('unit-test'), 'deployment environment missing');
  for (const s of ['host.name', 'fixture-host', 'container.id', 'c0ffee'.repeat(10) + 'beef', 'process.pid']) {
    assert.ok(traces.includes(s), `${s} missing from the resource`);
  }
  assert.match(traces, /fixture-host-\d+/, 'instance ID derives from host.name');
});

test('never exports credentials, cookies, bodies or command-line arguments', () => {
  for (const r of cap.received) {
    const payload = r.body.toString('latin1');
    for (const secret of SECRETS) {
      assert.ok(!payload.includes(secret), `${r.path} payload contains ${secret}`);
    }
  }
});

test('does not trace ignored health checks', () => {
  const traces = Buffer.concat(byPath('/v1/traces').map((r) => r.body)).toString('latin1');
  assert.ok(!traces.includes('/health'));
  const events = byPath('/api/v1/events/batch').flatMap((r) => JSON.parse(r.body.toString('utf8')).events);
  assert.ok(!events.some((e) => e.attributes['url.path'] === '/health'));
});

test('request events carry only method, path without query, and status', () => {
  const batches = byPath('/api/v1/events/batch');
  assert.ok(batches.length > 0, 'no events sent');
  for (const b of batches) assert.equal(b.headers.authorization, 'Bearer ingest-token-XYZ');
  const events = batches.flatMap((r) => JSON.parse(r.body.toString('utf8')).events);
  const started = events.filter((e) => e.event_type === 'request.started');
  const completed = events.filter((e) => e.event_type === 'request.completed');
  assert.equal(started.length, 1);
  assert.equal(completed.length, 1);
  assert.deepEqual(started[0].attributes, { 'http.request.method': 'POST', 'url.path': '/orders' });
  assert.deepEqual(completed[0].attributes, { 'http.response.status_code': '200' });
  assert.match(started[0].trace_id, /^[0-9a-f]{32}$/);
  assert.equal(started[0].trace_id, completed[0].trace_id);
  assert.equal(started[0].service.name, 'fixture-svc');
  const custom = events.find((e) => e.event_type === 'fixture.custom');
  assert.deepEqual(custom?.attributes, { stage: 'done' });
});

test('diagnostics report the endpoint without a trailing slash and no token', () => {
  const diag = JSON.parse(result.stdout.trim().split(/\r?\n/).at(-1));
  assert.equal(diag.serviceName, 'fixture-svc');
  assert.equal(diag.endpoint, cap.url);
  assert.ok(!JSON.stringify(diag).includes('ingest-token-XYZ'));
  assert.equal(diag.events.dropped, 0);
});
