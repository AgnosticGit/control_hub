// Read-only MCP server (`karbird mcp`) over stdio against the running API.
// Needs `just dev` and `just demo` running; builds the CLI if needed.
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { createInterface } from 'node:readline';
import { join } from 'node:path';
import { API, DEMO, ISO, Session, root, eventually } from './lib.mjs';

const bin = join(root, 'target', 'debug', process.platform === 'win32' ? 'karbird.exe' : 'karbird');

class McpClient {
  constructor(token) {
    this.proc = spawn(bin, ['mcp', '--api-url', API], {
      env: { ...process.env, KARBIRD_API_TOKEN: token },
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    this.nextId = 1;
    this.pending = new Map();
    this.stdout = [];
    createInterface({ input: this.proc.stdout }).on('line', (line) => {
      this.stdout.push(line);
      const msg = JSON.parse(line);
      this.pending.get(msg.id)?.(msg);
      this.pending.delete(msg.id);
    });
  }
  request(method, params) {
    const id = this.nextId++;
    this.proc.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
    return new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error(`${method} timed out`)), 20_000);
      this.pending.set(id, (m) => {
        clearTimeout(t);
        resolve(m);
      });
    });
  }
  notify(method) {
    this.proc.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method })}\n`);
  }
  async call(name, args = {}) {
    const r = await this.request('tools/call', { name, arguments: args });
    assert.ok(r.result, JSON.stringify(r));
    return r.result;
  }
  close() {
    this.proc.stdin.end();
    return new Promise((resolve) => this.proc.on('exit', resolve));
  }
}

let admin;
let token;
let tokenId;
let mcp;

before(async () => {
  const build = spawnSync('cargo', ['build', '-q', '-p', 'karbird-cli'], { cwd: root, stdio: 'inherit' });
  assert.equal(build.status, 0, 'cargo build karbird-cli');
  admin = new Session();
  await admin.login('admin@karbird.local');
  const created = await admin.mutate(`${DEMO}/tokens`, { kind: 'api', name: `mcp-${randomUUID().slice(0, 8)}`, expires_in_days: 1 });
  assert.equal(created.status, 200);
  token = created.body.secret;
  tokenId = created.body.token.id;
  mcp = new McpClient(token);
  const init = await mcp.request('initialize', {
    protocolVersion: '2025-06-18',
    capabilities: {},
    clientInfo: { name: 'integration-test', version: '1' },
  });
  assert.equal(init.result.protocolVersion, '2025-06-18');
  assert.match(init.result.instructions, /untrusted/);
  mcp.notify('notifications/initialized');
});

after(async () => {
  await mcp?.close();
});

test('only read-only tools are offered', async () => {
  const r = await mcp.request('tools/list', {});
  const names = r.result.tools.map((t) => t.name).sort();
  assert.deepEqual(names, [
    'get_context', 'get_coverage', 'get_edge_traffic', 'get_incident_snapshot', 'get_infrastructure',
    'get_infrastructure_timeline', 'get_timeline', 'get_topology', 'get_trace', 'list_action_plans', 'list_events', 'list_services',
    'search_logs', 'search_traces',
  ]);
  assert.ok(r.result.tools.every((t) => t.annotations.readOnlyHint === true && t.annotations.destructiveHint === false));
});

test('tools return scoped, bounded data with the untrusted-data notice', async () => {
  const services = await eventually(async () => {
    const s = await mcp.call('list_services', { minutes: 15 });
    assert.equal(s.isError, false, JSON.stringify(s));
    assert.ok(s.structuredContent.data.services.length > 0);
    return s;
  });
  assert.deepEqual(services.structuredContent.scope, { project_id: 'demo', environment_id: 'local-demo' });
  assert.match(services.structuredContent.notice, /untrusted/);
  const names = services.structuredContent.data.services.map((s) => s.name);
  assert.ok(names.includes('orders') && names.includes('gateway'), names.join(','));

  const ctx = await mcp.call('get_context', { minutes: 15 });
  assert.equal(ctx.isError, false);
  assert.ok(ctx.structuredContent.dependencies.some((e) => e.from === 'gateway' && e.to === 'orders'));

  const orders = services.structuredContent.data.services.find((s) => s.name === 'orders');
  const traces = await mcp.call('search_traces', { minutes: 15, service_id: orders.service_id, limit: 5 });
  assert.equal(traces.isError, false);
  const list = traces.structuredContent.data.traces;
  assert.ok(list.length > 0 && list.length <= 5);
  const trace = await mcp.call('get_trace', { trace_id: list[0].trace_id });
  assert.equal(trace.isError, false, JSON.stringify(trace.content));
  assert.ok(trace.structuredContent.data.spans.length > 0);
  const json = JSON.stringify(trace.structuredContent);
  assert.ok(!/authorization|cookie/i.test(json.replace(/"notice":"[^"]*"/, '')), 'no credentials in span attributes');

  const logs = await mcp.call('search_logs', { minutes: 15, limit: 3 });
  assert.equal(logs.isError, false);
  assert.ok(logs.structuredContent.data.logs.length <= 3);
  for (const l of logs.structuredContent.data.logs) assert.ok(!/[\u0000-\u001f]/.test(l.body ?? ''), 'control characters stripped');

  const plans = await mcp.call('list_action_plans');
  assert.equal(plans.isError, false);
  assert.ok(Array.isArray(plans.structuredContent.data.plans));
});

test('infrastructure and edge traffic stay read-only and environment-bound', async () => {
  const at = String(BigInt(Date.now()) * 1_000_000n);
  const infrastructure = await eventually(async () => {
    const result = await mcp.call('get_infrastructure', { at_ns: at });
    assert.equal(result.isError, false, JSON.stringify(result));
    assert.ok(Array.isArray(result.structuredContent.data.hosts));
    return result;
  });
  assert.equal(infrastructure.structuredContent.at_ns, at);
  assert.match(infrastructure.structuredContent.ui_path, /^\/app\?project=demo&environment=local-demo&at_ns=/);

  const topology = await mcp.call('get_topology', { minutes: 15 });
  assert.equal(topology.isError, false, JSON.stringify(topology));
  const saved = topology.structuredContent.data;
  assert.equal(saved.geometry.version, 3);
  assert.ok(saved.geometry.revision > 0);
  const reopened = await mcp.call('get_topology', { from_ns: saved.from_ns, to_ns: saved.to_ns });
  assert.equal(reopened.isError, false, JSON.stringify(reopened));
  assert.deepEqual(reopened.structuredContent.data.geometry, saved.geometry, 'MCP reuses the canonical backend map');
  const edge = topology.structuredContent.data.edges.find((candidate) => candidate.source && candidate.target && candidate.relation);
  assert.ok(edge, 'a demo edge is available');
  const traffic = await mcp.call('get_edge_traffic', {
    source: edge.source,
    target: edge.target,
    relation: edge.relation,
    from_ns: String(BigInt(at) - 60_000_000_000n),
    to_ns: at,
    buckets: 6,
  });
  assert.equal(traffic.isError, false, JSON.stringify(traffic));
  assert.ok(Object.hasOwn(traffic.structuredContent.data, 'avg_request_bps'));
  assert.ok(Object.hasOwn(traffic.structuredContent.data, 'size_coverage'));

  const iso = new Session();
  await iso.login('isolation@karbird.local');
  const isoTok = await iso.mutate(`${ISO}/tokens`, { kind: 'api', name: `mcp-infra-${randomUUID().slice(0, 8)}`, expires_in_days: 1 });
  assert.equal(isoTok.status, 200);
  const other = new McpClient(isoTok.body.secret);
  let isolatedTraffic;
  try {
    await other.request('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 't', version: '1' } });
    isolatedTraffic = await other.call('get_edge_traffic', {
      source: edge.source,
      target: edge.target,
      relation: edge.relation,
      from_ns: String(BigInt(at) - 60_000_000_000n),
      to_ns: at,
    });
  } finally {
    await other.close();
  }
  assert.equal(isolatedTraffic.isError, false, JSON.stringify(isolatedTraffic));
  assert.equal(isolatedTraffic.structuredContent.data.total_calls, 0, 'demo edge IDs have no calls in another environment');
});

test('timeline, coverage and incident snapshots preserve exact time ranges', async () => {
  const toNs = BigInt(Date.now()) * 1_000_000n;
  const fromNs = toNs - 60_000_000_000n;
  const time = { from_ns: String(fromNs), to_ns: String(toNs) };

  const timeline = await mcp.call('get_timeline', { ...time, limit: 20 });
  assert.equal(timeline.isError, false, JSON.stringify(timeline));
  assert.deepEqual([timeline.structuredContent.from_ns, timeline.structuredContent.to_ns], [time.from_ns, time.to_ns]);
  assert.ok(Array.isArray(timeline.structuredContent.data.items));
  assert.ok(timeline.structuredContent.data.completeness);

  const coverage = await mcp.call('get_coverage', time);
  assert.equal(coverage.isError, false, JSON.stringify(coverage));
  assert.ok(Array.isArray(coverage.structuredContent.data.services));
  assert.match(coverage.structuredContent.ui_path, /from_ns=/);

  const services = await mcp.call('list_services', time);
  assert.equal(services.isError, false, JSON.stringify(services));
  const orders = services.structuredContent.data.services.find((service) => service.name === 'orders');
  assert.ok(orders?.service_id, 'orders is present with an opaque service ID');
  const snapshot = await mcp.call('get_incident_snapshot', { ...time, service_id: orders.service_id });
  assert.equal(snapshot.isError, false, JSON.stringify(snapshot));
  assert.match(snapshot.structuredContent.notice, /untrusted/);
  for (const key of ['services', 'topology', 'traces', 'logs', 'events', 'coverage', 'infrastructure']) {
    assert.ok(snapshot.structuredContent.data[key], `snapshot includes ${key}`);
  }

  const inverted = await mcp.call('get_coverage', { from_ns: time.to_ns, to_ns: time.from_ns });
  assert.equal(inverted.isError, true);
  const incomplete = await mcp.call('get_timeline', { from_ns: time.from_ns });
  assert.equal(incomplete.isError, true);
});

test('invalid arguments and unknown tools are refused', async () => {
  const bad = await mcp.call('get_trace', { trace_id: '../../tokens' });
  assert.equal(bad.isError, true);
  const extra = await mcp.call('list_services', { environment_id: 'isolation-test' });
  assert.equal(extra.isError, true, 'scope cannot be chosen by the caller');
  const unknown = await mcp.request('tools/call', { name: 'propose_action', arguments: {} });
  assert.equal(unknown.error.code, -32602);
  assert.ok(mcp.stdout.every((l) => !l.includes(token)), 'token never echoed');
});

test('another tenant cannot be reached and revocation takes effect', async () => {
  const iso = new Session();
  await iso.login('isolation@karbird.local');
  const isoTok = await iso.mutate(`${ISO}/tokens`, { kind: 'api', name: `mcp-iso-${randomUUID().slice(0, 8)}`, expires_in_days: 1 });
  assert.equal(isoTok.status, 200);
  const other = new McpClient(isoTok.body.secret);
  let s;
  try {
    await other.request('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 't', version: '1' } });
    s = await other.call('list_services');
  } finally {
    await other.close();
  }
  assert.equal(s.isError, false);
  assert.deepEqual(s.structuredContent.scope, { project_id: 'isolation', environment_id: 'isolation-test' });
  assert.ok(!s.structuredContent.data.services.some((x) => x.name === 'orders'), 'no demo data in another tenant');

  assert.equal((await admin.mutate(`${DEMO}/tokens/${tokenId}/revoke`, {})).status, 200);
  const r = await mcp.call('list_services');
  assert.equal(r.isError, true);
  assert.match(r.content[0].text, /rejected/);
});
