// External clients (requests into the project from outside) and external services (HTTP calls out of it)
// must show up in the topology as distinct, inferred nodes — and internal HTTP calls must not.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Session, ISO, DEMO, eventually, encodeTraces, otlpHttp, nowNs, hex, API } from './lib.mjs';

const run = hex(4);
const front = `front-${run}`;
const back = `back-${run}`;
const extHost = `api.ext-${run}.test`;
const internalHost = `${back}.internal`;
const traceId = hex(16);
const [rootId, extCallId, intCallId, lateCallId, backId] = [hex(8), hex(8), hex(8), hex(8), hex(8)];

const iso = new Session();
let sent = false;

async function sendTrace() {
  if (sent) return;
  const t = nowNs() - 5_000_000_000n;
  const ms = 1_000_000n;
  const frontSpans = [
    { traceId, spanId: rootId, name: 'GET /checkout', kind: 2, startNs: t, endNs: t + 40n * ms, attributes: { 'http.request.method': 'GET' } },
    {
      traceId,
      spanId: extCallId,
      parentSpanId: rootId,
      name: 'GET',
      kind: 3,
      startNs: t + 2n * ms,
      endNs: t + 12n * ms,
      attributes: { 'http.request.method': 'GET', 'server.address': extHost, 'url.full': `https://${extHost}/v1/rates?key=secret` },
    },
    {
      traceId,
      spanId: intCallId,
      parentSpanId: rootId,
      name: 'POST',
      kind: 3,
      startNs: t + 14n * ms,
      endNs: t + 30n * ms,
      attributes: { 'http.request.method': 'POST', 'server.address': internalHost },
    },
    // A second call to the same internal host whose server span has not arrived (yet): services
    // export in batches, so this is the normal state for the newest calls.
    {
      traceId,
      spanId: lateCallId,
      parentSpanId: rootId,
      name: 'POST',
      kind: 3,
      startNs: t + 31n * ms,
      endNs: t + 38n * ms,
      attributes: { 'http.request.method': 'POST', 'server.address': internalHost },
    },
  ];
  const backSpans = [
    { traceId, spanId: backId, parentSpanId: intCallId, name: 'POST /orders', kind: 2, startNs: t + 15n * ms, endNs: t + 29n * ms },
  ];
  for (const [svc, spans] of [[front, frontSpans], [back, backSpans]]) {
    const r = await otlpHttp('traces', encodeTraces(svc, spans));
    assert.equal(r.status, 200, `ingest ${svc}`);
  }
  sent = true;
}

async function topology() {
  const { status, body } = await iso.json(`${ISO}/topology`);
  assert.equal(status, 200);
  const byId = new Map(body.nodes.map((n) => [n.id, n]));
  const service = (name) => body.nodes.find((n) => n.kind === 'service' && n.name === name);
  const edgesInto = (id) => body.edges.filter((e) => e.target === id).map((e) => ({ ...e, from: byId.get(e.source) }));
  const edgesFrom = (id) => body.edges.filter((e) => e.source === id).map((e) => ({ ...e, to: byId.get(e.target) }));
  return { body, service, edgesInto, edgesFrom };
}

test('setup: isolation user session and one trace with an external caller and callee', async () => {
  await iso.login('isolation@karbird.local');
  await sendTrace();
});

test('an uninstrumented caller is drawn as the inferred "External clients" node', async () => {
  await sendTrace();
  await eventually(async () => {
    const { service, edgesInto } = await topology();
    const a = service(front);
    assert.ok(a, `service ${front} not in topology yet`);
    const incoming = edgesInto(a.id).filter((e) => e.from?.kind === 'external_client');
    assert.equal(incoming.length, 1, `edges into ${front}: ${JSON.stringify(edgesInto(a.id))}`);
    const [edge] = incoming;
    assert.equal(edge.from.name, 'External clients');
    assert.equal(edge.from.provenance, 'inferred');
    assert.equal(edge.relation, 'calls');
    assert.equal(edge.provenance, 'inferred');

    // back is called by an instrumented service, so it has no external caller.
    const b = service(back);
    assert.ok(b, `service ${back} not in topology yet`);
    assert.deepEqual(edgesInto(b.id).filter((e) => e.from?.kind === 'external_client'), []);
  }, { timeoutMs: 45_000 });
});

test('an HTTP call to a host outside the project is an external service; an internal call is not', async () => {
  await sendTrace();
  await eventually(async () => {
    const { body, service, edgesFrom } = await topology();
    const a = service(front);
    assert.ok(a);
    const ext = edgesFrom(a.id).filter((e) => e.to?.kind === 'external_dependency');
    assert.deepEqual(ext.map((e) => e.to.name), [extHost], `edges from ${front}: ${JSON.stringify(edgesFrom(a.id))}`);
    assert.equal(ext[0].to.provenance, 'inferred');
    assert.ok(
      !body.nodes.some((n) => n.name === internalHost),
      'a host answered by an instrumented service is internal, even for calls whose answer has not arrived yet',
    );
    assert.ok(!body.nodes.some((n) => n.name.includes('key=secret') || n.name.includes('/v1/rates')), 'only the host is kept, never the path or query');
  }, { timeoutMs: 45_000 });
});

test('timeline marks only the root server span as coming from the external clients node', async () => {
  await sendTrace();
  await eventually(async () => {
    const { body: topo } = await iso.json(`${ISO}/topology`);
    const clients = topo.nodes.find((n) => n.kind === 'external_client');
    assert.ok(clients);
    const { status, body } = await iso.json(`${ISO}/timeline?limit=20000`);
    assert.equal(status, 200);
    const spans = body.items.filter((i) => i.type === 'span' && i.trace_id === traceId);
    assert.equal(spans.length, 5, `spans of trace: ${spans.length}`);
    const withSource = spans.filter((s) => s.source_entity_id);
    assert.deepEqual(withSource.map((s) => s.span_id), [rootId]);
    assert.equal(withSource[0].source_entity_id, clients.id);
  }, { timeoutMs: 45_000 });
});

test('external nodes are tenant-scoped and need a session', async () => {
  await sendTrace();
  const admin = new Session();
  await admin.login('admin@karbird.local');
  const demo = await admin.json(`${DEMO}/topology`);
  assert.equal(demo.status, 200);
  assert.ok(!demo.body.nodes.some((n) => n.name === extHost || n.name === front), 'isolation data must not leak into demo');
  const forbidden = await admin.json(`${ISO}/topology`);
  assert.equal(forbidden.status, 404);

  const anon = await fetch(`${API}${ISO}/topology`);
  assert.equal(anon.status, 401);
  const anonTimeline = await fetch(`${API}${ISO}/timeline`);
  assert.equal(anonTimeline.status, 401);
});
