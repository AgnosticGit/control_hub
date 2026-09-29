// Ingestion surface: OTLP/HTTP protobuf, OTLP/gRPC, the events API, size/type limits,
// server-side redaction end to end (ingest -> Kafka -> processor -> ClickHouse -> API) and
// the ingest self-metrics. Writes only into the isolation tenant's `isolation-test`
// environment so the demo map is not polluted.
// Run: node --test "tests/integration/*.test.mjs"
import { test, before } from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { gzipSync } from 'node:zlib';
import {
  ADMIN_METRICS,
  OTLP,
  creds,
  Session,
  ISO,
  eventually,
  encodeTraces,
  encodeLogs,
  decodeFields,
  otlpHttp,
  grpcUnary,
  ingestMetrics,
  postOversized,
  hex,
  nowNs,
} from './lib.mjs';

const TOKEN = creds.ingest_tokens['isolation-test'];
const TRACE_SVC = 'opentelemetry.proto.collector.trace.v1.TraceService';
const LOGS_SVC = 'opentelemetry.proto.collector.logs.v1.LogsService';
const marker = (p) => `${p}-${randomUUID().slice(0, 8)}`;

let iso;
before(async () => {
  iso = new Session();
  await iso.login('isolation@controlhub.local');
});

async function traceDetail(traceId) {
  return eventually(
    async () => {
      const r = await iso.json(`${ISO}/traces/${traceId}`);
      assert.equal(r.status, 200, `trace ${traceId} not visible yet (${r.status})`);
      return r.body;
    },
    { timeoutMs: 45_000 },
  );
}

async function logsMatching(q) {
  return eventually(
    async () => {
      const r = await iso.json(`${ISO}/logs?q=${encodeURIComponent(q)}`);
      assert.equal(r.status, 200);
      assert.ok(r.body.logs.length > 0, `no logs for ${q} yet`);
      return r.body.logs;
    },
    { timeoutMs: 45_000 },
  );
}

function span(overrides = {}) {
  const end = nowNs();
  return { traceId: hex(16), spanId: hex(8), name: 'GET /it', startNs: end - 3_000_000n, endNs: end, ...overrides };
}

test('ingest health endpoint answers without a token', async () => {
  const res = await fetch(`${OTLP}/health`);
  assert.equal(res.status, 200);
  assert.equal(await res.text(), 'ok');
});

test('OTLP/HTTP protobuf traces are accepted, processed and queryable with redaction applied', async () => {
  const svc = marker('it-redact');
  const s = span({
    kind: 3,
    attributes: {
      'http.request.header.authorization': 'Bearer super-secret-token',
      'http.request.header.cookie': 'sid=abc',
      'user.password': 'hunter2',
      'app.api_key': 'sk-live-123',
      'db.statement': 'SELECT * FROM users WHERE email = \'a@b.c\'',
      'db.query.text': 'DELETE FROM orders',
      'http.request.body': '{"card":"4111"}',
      'http.route': '/checkout',
      'note': 'forwarded Bearer leaked-token-1 upstream',
    },
    status: { code: 2, message: 'upstream said Basic dXNlcjpwYXNz' },
  });
  const r = await otlpHttp('traces', encodeTraces(svc, [s], { 'deployment.secret': 'resource-secret' }));
  assert.equal(r.status, 200);
  assert.equal(r.contentType, 'application/x-protobuf');
  assert.deepEqual(decodeFields(r.body), {}, 'full success has no partial_success');

  const detail = await traceDetail(s.traceId);
  const got = detail.spans.find((x) => x.span_id === s.spanId);
  assert.ok(got, 'span stored');
  assert.equal(got.service_name, svc);
  assert.equal(got.status_code, 2);
  const a = got.attributes;
  assert.equal(a['http.route'], '/checkout', 'harmless attributes are kept');
  for (const k of ['http.request.header.authorization', 'http.request.header.cookie', 'user.password', 'app.api_key']) {
    assert.equal(a[k], '[REDACTED]', `${k} must be redacted`);
  }
  for (const k of ['db.statement', 'db.query.text', 'http.request.body']) assert.ok(!(k in a), `${k} must be dropped`);
  assert.equal(a.note, 'forwarded Bearer [REDACTED] upstream', 'bearer credentials in free text are masked');
  assert.equal(got.status_message, 'upstream said Basic [REDACTED]');
  const raw = JSON.stringify(detail);
  for (const secret of ['super-secret-token', 'hunter2', 'sk-live-123', 'sid=abc', 'SELECT *', '4111', 'leaked-token-1', 'dXNlcjpwYXNz', 'resource-secret']) {
    assert.ok(!raw.includes(secret), `secret leaked into the API: ${secret}`);
  }
});

test('OTLP/HTTP protobuf logs are masked, correlated to their trace and returned as plain data', async () => {
  const svc = marker('it-logs');
  const s = span();
  await otlpHttp('traces', encodeTraces(svc, [s]));
  const body = `${svc} login failed: Authorization: Bearer eyJhbGciOi.payload.sig <script>alert(1)</script>`;
  const r = await otlpHttp(
    'logs',
    encodeLogs(svc, [
      {
        timeNs: nowNs(),
        severity: 17,
        severityText: 'ERROR',
        body,
        traceId: s.traceId,
        spanId: s.spanId,
        attributes: { 'session.id': 'abc', 'user.id': 'u-1', 'http.request.header.x-csrf-token': 'chc_x' },
      },
    ]),
  );
  assert.equal(r.status, 200);
  const [log] = await logsMatching(svc);
  assert.equal(log.service_name, svc);
  assert.equal(log.severity_number, 17);
  assert.equal(log.trace_id, s.traceId);
  assert.equal(log.span_id, s.spanId);
  assert.ok(log.body.includes('Bearer [REDACTED]'), log.body);
  assert.ok(!log.body.includes('eyJhbGciOi'), 'token in log body is masked');
  assert.ok(log.body.includes('<script>alert(1)</script>'), 'markup is stored verbatim as data (escaping is the UI’s job)');
  assert.equal(log.attributes['session.id'], '[REDACTED]');
  assert.equal(log.attributes['http.request.header.x-csrf-token'], '[REDACTED]');
  assert.equal(log.attributes['user.id'], 'u-1');
  const detail = await traceDetail(s.traceId);
  assert.ok(detail.logs.some((l) => l.id === log.id), 'log is correlated in the trace detail');
});

test('invalid spans are rejected individually with an OTLP partial success', async () => {
  const good = span();
  const zeroTrace = span({ traceId: '0'.repeat(32) });
  const endBeforeStart = span({ startNs: nowNs(), endNs: nowNs() - 1_000_000_000n });
  const shortSpanId = { ...span(), spanId: 'abcd' };
  const r = await otlpHttp('traces', encodeTraces(marker('it-partial'), [good, zeroTrace, endBeforeStart, shortSpanId]));
  assert.equal(r.status, 200);
  const partial = decodeFields(decodeFields(r.body)[1][0]);
  assert.equal(partial[1][0], 3n, 'rejected_spans');
  assert.match(partial[2][0].toString(), /rejected/);
  await traceDetail(good.traceId);
});

test('logs without any timestamp or with malformed IDs are rejected and counted', async () => {
  const before = await ingestMetrics();
  const r = await otlpHttp(
    'logs',
    encodeLogs(marker('it-badlog'), [
      { timeNs: 0n, body: 'no time' },
      { timeNs: nowNs(), body: 'bad trace id', traceId: 'abcd' },
    ]),
  );
  assert.equal(r.status, 200);
  const partial = decodeFields(decodeFields(r.body)[1][0]);
  assert.equal(partial[1][0], 2n, 'rejected_log_records');
  const after = await ingestMetrics();
  assert.ok(
    after['controlhub_ingest_items_total{signal="logs",result="rejected"}'] >= before['controlhub_ingest_items_total{signal="logs",result="rejected"}'] + 2,
    'every rejected record increments a counter',
  );
});

test('gzip-encoded OTLP bodies are accepted; invalid gzip is 400; unknown encodings are 415', async () => {
  const s = span();
  const gz = await otlpHttp('traces', gzipSync(encodeTraces(marker('it-gzip'), [s])), { headers: { 'content-encoding': 'gzip' } });
  assert.equal(gz.status, 200);
  await traceDetail(s.traceId);
  const badGz = await otlpHttp('traces', Buffer.from('not gzip at all'), { headers: { 'content-encoding': 'gzip' } });
  assert.equal(badGz.status, 400);
  const br = await otlpHttp('traces', encodeTraces('x', [span()]), { headers: { 'content-encoding': 'br' } });
  assert.equal(br.status, 415);
});

test('OTLP/HTTP content types: protobuf only (JSON and others are 415, never silently accepted)', async () => {
  for (const ct of ['application/json', 'text/plain', 'application/octet-stream', '']) {
    for (const signal of ['traces', 'logs']) {
      const r = await otlpHttp(signal, Buffer.from('{}'), { headers: { 'content-type': ct } });
      assert.equal(r.status, 415, `${signal} with "${ct}"`);
      assert.equal(r.contentType, 'application/x-protobuf', 'errors are google.rpc.Status protobuf');
      const status = decodeFields(r.body);
      assert.equal(status[1]?.[0], 3n, 'gRPC code INVALID_ARGUMENT');
    }
  }
  const withParams = await otlpHttp('traces', encodeTraces(marker('it-ct'), [span()]), {
    headers: { 'content-type': 'application/x-protobuf; charset=binary' },
  });
  assert.equal(withParams.status, 200, 'media type parameters are ignored');
});

test('malformed protobuf is 400 and counted; metrics export is an explicit 501', async () => {
  const before = await ingestMetrics();
  const r = await otlpHttp('traces', Buffer.from([0x0a, 0xff, 0xff, 0xff, 0xff, 0x0f, 0x01]));
  assert.equal(r.status, 400);
  const after = await ingestMetrics();
  assert.ok(after['controlhub_ingest_requests_total{result="rejected_invalid"}'] > before['controlhub_ingest_requests_total{result="rejected_invalid"}']);
  const m = await fetch(`${OTLP}/v1/metrics`, {
    method: 'POST',
    headers: { authorization: `Bearer ${TOKEN}`, 'content-type': 'application/x-protobuf' },
    body: new Uint8Array(),
  });
  assert.equal(m.status, 501);
});

test('OTLP/HTTP bodies over 4 MiB are refused with 413', async () => {
  const status = await postOversized(`${OTLP}/v1/traces`, { authorization: `Bearer ${TOKEN}`, 'content-type': 'application/x-protobuf' }, 5 * 1024 * 1024);
  assert.equal(status, 413);
  const events = await postOversized(`${OTLP}/api/v1/events/batch`, { authorization: `Bearer ${TOKEN}`, 'content-type': 'application/json' }, 5 * 1024 * 1024);
  assert.equal(events, 413);
});

test('ingestion tokens are checked before anything else and API tokens are not ingestion tokens', async () => {
  const body = encodeTraces('x', [span()]);
  for (const token of ['', 'chi_invalid', 'cha_not-for-ingest', creds.ingest_tokens['isolation-test'].slice(0, -2)]) {
    const r = await fetch(`${OTLP}/v1/traces`, {
      method: 'POST',
      headers: { ...(token ? { authorization: `Bearer ${token}` } : {}), 'content-type': 'application/x-protobuf' },
      body,
    });
    assert.equal(r.status, 401, `token "${token.slice(0, 6)}…"`);
  }
  const events = await fetch(`${OTLP}/api/v1/events/batch`, {
    method: 'POST',
    headers: { authorization: 'Basic Y2hpOng=', 'content-type': 'application/json' },
    body: '{"events":[]}',
  });
  assert.equal(events.status, 401);
});

test('OTLP/gRPC trace and log export work end to end with redaction', async () => {
  const svc = marker('it-grpc');
  const s = span({ attributes: { 'http.request.header.authorization': 'Bearer grpc-secret', 'http.route': '/grpc' } });
  const meta = { authorization: `Bearer ${TOKEN}` };
  const t = await grpcUnary(TRACE_SVC, 'Export', encodeTraces(svc, [s]), meta);
  assert.equal(t.status, 0, `grpc-status ${t.status} ${t.message}`);
  const l = await grpcUnary(LOGS_SVC, 'Export', encodeLogs(svc, [{ timeNs: nowNs(), body: `${svc} via grpc`, traceId: s.traceId }]), meta);
  assert.equal(l.status, 0, `grpc-status ${l.status} ${l.message}`);
  const detail = await traceDetail(s.traceId);
  const got = detail.spans.find((x) => x.span_id === s.spanId);
  assert.equal(got.attributes['http.request.header.authorization'], '[REDACTED]');
  assert.equal(got.attributes['http.route'], '/grpc');
  await logsMatching(`${svc} via grpc`);
});

test('OTLP/gRPC rejects missing/invalid tokens (UNAUTHENTICATED) and reports partial success', async () => {
  const body = encodeTraces('x', [span()]);
  const none = await grpcUnary(TRACE_SVC, 'Export', body);
  assert.equal(none.status, 16, `no token: ${none.status} ${none.message}`);
  assert.equal((await grpcUnary(TRACE_SVC, 'Export', body, { authorization: 'Bearer chi_invalid' })).status, 16);
  const logs = encodeLogs('x', [{ timeNs: nowNs(), body: 'x' }]);
  assert.equal((await grpcUnary(LOGS_SVC, 'Export', logs, { authorization: 'Bearer chi_invalid' })).status, 16);
  const partial = await grpcUnary(TRACE_SVC, 'Export', encodeTraces(marker('it-grpc-partial'), [span(), span({ traceId: '0'.repeat(32) })]), {
    authorization: `Bearer ${TOKEN}`,
  });
  assert.equal(partial.status, 0);
  assert.equal(decodeFields(decodeFields(partial.body)[1][0])[1][0], 1n, 'rejected_spans over gRPC');
  const unknown = await grpcUnary(TRACE_SVC, 'Nope', body, { authorization: `Bearer ${TOKEN}` });
  assert.equal(unknown.status, 12, 'UNIMPLEMENTED for unknown methods');
});

function event(overrides = {}) {
  return {
    event_id: randomUUID(),
    event_type: 'deployment.finished',
    service: { name: 'it-events' },
    event_time: `${nowNs()}`,
    ...overrides,
  };
}

async function sendEvents(body, { contentType = 'application/json', token = TOKEN } = {}) {
  const res = await fetch(`${OTLP}/api/v1/events/batch`, {
    method: 'POST',
    headers: { authorization: `Bearer ${token}`, 'content-type': contentType },
    body: typeof body === 'string' ? body : JSON.stringify(body),
  });
  return { status: res.status, body: await res.json().catch(() => null) };
}

test('events API validates every event and reports rejections by index', async () => {
  const svc = marker('it-ev');
  const r = await sendEvents({
    events: [
      event({ service: { name: svc } }),
      event({ event_type: 'Bad Type' }),
      event({ event_id: '' }),
      event({ event_time: '-5' }),
      event({ event_time: 'soon' }),
      event({ trace_id: 'XYZ' }),
      event({ span_id: 'abc' }),
      event({ service: { name: '' } }),
      event({ payload: { blob: 'x'.repeat(9000) } }),
      { nonsense: true },
    ],
  });
  assert.equal(r.status, 200, JSON.stringify(r.body));
  assert.equal(r.body.accepted, 1);
  assert.equal(r.body.durability, 'broker_acknowledged');
  assert.deepEqual(
    r.body.rejected.map((x) => x.index),
    [1, 2, 3, 4, 5, 6, 7, 8, 9],
  );
  for (const x of r.body.rejected) assert.ok(x.reason.length > 0);
});

test('events API envelope errors: all-invalid 400, malformed JSON 400, too many 400, wrong type 415', async () => {
  const allBad = await sendEvents({ events: [event({ event_type: 'NOPE' })] });
  assert.equal(allBad.status, 400);
  assert.equal(allBad.body.accepted, 0);
  assert.equal(allBad.body.rejected.length, 1);
  assert.equal((await sendEvents('{"events":')).status, 400);
  assert.equal((await sendEvents('{"not_events":[]}')).status, 400);
  const tooMany = await sendEvents({ events: Array.from({ length: 1001 }, () => event()) });
  assert.equal(tooMany.status, 400);
  assert.match(tooMany.body.error, /at most/);
  assert.equal((await sendEvents({ events: [event()] }, { contentType: 'text/plain' })).status, 415);
  assert.equal((await sendEvents({ events: [event()] }, { token: 'chi_invalid' })).status, 401);
});

test('event attributes are redacted and the scope comes from the token', async () => {
  const svc = marker('it-evred');
  const traceId = hex(16);
  const r = await sendEvents({
    events: [
      event({
        service: { name: svc },
        trace_id: traceId,
        attributes: {
          password: 'p@ss',
          'auth.token': 'tok',
          'deploy.version': '1.2.3',
          note: 'Bearer ev-secret',
          count: 3,
          tenant_id: 'local',
          project_id: 'demo',
        },
      }),
    ],
  });
  assert.equal(r.status, 200, JSON.stringify(r.body));
  const ev = await eventually(
    async () => {
      const q = await iso.json(`${ISO}/events?trace_id=${traceId}`);
      assert.equal(q.status, 200);
      const found = q.body.events.find((e) => e.service_name === svc);
      assert.ok(found, 'event visible');
      return found;
    },
    { timeoutMs: 45_000 },
  );
  assert.equal(ev.trace_id, traceId);
  assert.equal(ev.attributes.password, '[REDACTED]');
  assert.equal(ev.attributes['auth.token'], '[REDACTED]');
  assert.equal(ev.attributes['deploy.version'], '1.2.3');
  assert.equal(ev.attributes.note, 'Bearer [REDACTED]');
  assert.equal(ev.attributes.count, '3', 'non-string values are stringified');
  const admin = new Session();
  await admin.login('admin@controlhub.local');
  const demo = await admin.json(`/api/v1/projects/demo/environments/local-demo/events?trace_id=${traceId}`);
  assert.equal(demo.status, 200);
  assert.equal(demo.body.events.length, 0, 'payload tenant attributes do not move the event');
});

test('ingest self-metrics are exposed in Prometheus text format and move with traffic', async () => {
  const res = await fetch(`${ADMIN_METRICS}/metrics`);
  assert.equal(res.status, 200);
  const text = await res.text();
  for (const name of [
    'controlhub_ingest_requests_total{result="accepted"}',
    'controlhub_ingest_items_total{signal="spans",result="accepted"}',
    'controlhub_ingest_attributes_redacted_total',
    'controlhub_ingest_publish_failures_total',
  ]) {
    assert.ok(text.includes(name), `missing metric ${name}`);
  }
  const before = await ingestMetrics();
  const r = await otlpHttp('traces', encodeTraces(marker('it-metrics'), [span({ attributes: { password: 'x', token: 'y' } })]));
  assert.equal(r.status, 200);
  const after = await ingestMetrics();
  assert.ok(after['controlhub_ingest_items_total{signal="spans",result="accepted"}'] > before['controlhub_ingest_items_total{signal="spans",result="accepted"}']);
  assert.ok(after.controlhub_ingest_attributes_redacted_total >= before.controlhub_ingest_attributes_redacted_total + 2);
});
