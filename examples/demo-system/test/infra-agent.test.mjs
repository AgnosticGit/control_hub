import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildResources, createHostState, parsePlacement, stepHost, syntheticSample } from '../src/infra-agent.mjs';
import { encodeMetrics } from '../src/otlp-metrics.mjs';
import { captureServer, cleanEnv, runNode, src, startNode } from './helpers.mjs';

const MIB = 1024 * 1024;
const ID = (c) => c.repeat(64);

const placement = (url = 'http://127.0.0.1:1') => [
  {
    host: 'vps-app',
    cores: 1,
    memoryMb: 1024,
    diskGb: 25,
    containers: [
      { name: 'orders', id: ID('a'), url },
      { name: 'inventory', id: ID('b'), url },
    ],
  },
];

/** Top-level protobuf fields: { field: [Buffer | bigint] } (varint, fixed64 and bytes). */
function fields(buf) {
  const out = {};
  let i = 0;
  const varint = () => {
    let shift = 0n;
    let v = 0n;
    for (;;) {
      const b = buf[i++];
      v |= BigInt(b & 0x7f) << shift;
      if (!(b & 0x80)) return v;
      shift += 7n;
    }
  };
  while (i < buf.length) {
    const key = Number(varint());
    const wire = key & 7;
    let value;
    if (wire === 0) value = varint();
    else if (wire === 1) {
      value = buf.subarray(i, i + 8);
      i += 8;
    } else if (wire === 2) {
      const len = Number(varint());
      value = buf.subarray(i, i + len);
      i += len;
    } else throw new Error(`wire type ${wire}`);
    (out[key >> 3] ??= []).push(value);
  }
  return out;
}

test('placement must name machines with positive sizes and 64-hex container IDs', () => {
  assert.equal(parsePlacement(JSON.stringify(placement())).length, 1);
  for (const bad of [
    [],
    [{ host: '', cores: 1, memoryMb: 1, diskGb: 1, containers: [] }],
    [{ host: 'h', cores: 0, memoryMb: 1, diskGb: 1, containers: [] }],
    [{ host: 'h', cores: 1, memoryMb: 1, diskGb: 1, containers: [{ name: 'x', id: 'short', url: 'u' }] }],
  ]) {
    assert.throws(() => parsePlacement(JSON.stringify(bad)), JSON.stringify(bad));
  }
});

test('a container has either a url or a synthetic load, and image/service are non-empty strings', () => {
  const host = (containers) => [{ host: 'h', cores: 1, memoryMb: 1, diskGb: 1, containers }];
  const pg = { name: 'postgres', id: ID('c'), image: 'postgres:16-alpine', service: 'db', synthetic: { cores: 0.03, rssMb: 96 } };
  assert.equal(parsePlacement(JSON.stringify(host([pg])))[0].containers[0].image, 'postgres:16-alpine');
  for (const bad of [
    { name: 'x', id: ID('c') },
    { ...pg, url: 'http://127.0.0.1:1' },
    { ...pg, synthetic: { cores: 0, rssMb: 96 } },
    { ...pg, synthetic: { cores: 0.1 } },
    { ...pg, image: '' },
    { ...pg, service: 5 },
  ]) {
    assert.throws(() => parsePlacement(JSON.stringify(host([bad]))), JSON.stringify(bad));
  }
});

test('a synthetic container reports cumulative CPU at its steady load and a fixed RSS', () => {
  assert.deepEqual(syntheticSample({ cores: 0.03, rssMb: 96 }, 100), { cpuMicros: 3_000_000, rssBytes: 96 * MIB });
  assert.equal(syntheticSample({ cores: 0.5, rssMb: 1 }, -5).cpuMicros, 0, 'never negative');
});

test('image and Compose service go into the container resource only when known', () => {
  const states = [
    createHostState({
      ...placement()[0],
      containers: [
        { name: 'orders', id: ID('a'), url: 'u' },
        { name: 'postgres', id: ID('c'), image: 'postgres:16-alpine', service: 'db', synthetic: { cores: 0.03, rssMb: 96 } },
      ],
    }),
  ];
  const samples = new Map([
    [ID('a'), { cpuMicros: 1, rssBytes: 1 }],
    [ID('c'), syntheticSample({ cores: 0.03, rssMb: 96 }, 10)],
  ]);
  const [, orders, pg] = buildResources(states, samples, 1n, 2n);
  assert.equal(pg.attributes['container.image.name'], 'postgres:16-alpine');
  assert.equal(pg.attributes['docker.compose.service'], 'db');
  assert.ok(!('container.image.name' in orders.attributes) && !('docker.compose.service' in orders.attributes));
});

test('a machine is never busier than its cores, and a hog shows as a full machine', () => {
  const s = createHostState(placement()[0]);
  assert.ok(stepHost(s, 10, 0) < 0.05, 'idle machine has only the base load');
  assert.ok(Math.abs(stepHost(s, 10, 9.5) - 0.98) < 1e-9);
  assert.equal(stepHost(s, 10, 50), 1);
  assert.ok(Math.abs(s.busySeconds + s.idleSeconds - 30) < 1e-9, 'busy + idle = cores × elapsed');
  assert.ok(s.rxBytes > s.txBytes && s.txBytes > 0);
});

test('resources carry host and container metrics under the agent metric names', () => {
  const states = placement().map(createHostState);
  stepHost(states[0], 10, 9);
  const samples = new Map([[ID('b'), { cpuMicros: 9_000_000, rssBytes: 2000 * MIB }]]);
  const [host, inventory, ...rest] = buildResources(states, samples, 1n, 2n);
  assert.equal(rest.length, 0, 'containers without a sample are left out');
  assert.deepEqual(host.attributes, { 'host.name': 'vps-app', 'host.id': 'vps-app', 'os.type': 'linux' });
  assert.deepEqual(
    host.metrics.map((m) => m.name),
    ['system.cpu.time', 'system.cpu.logical.count', 'system.memory.usage', 'system.memory.limit', 'system.filesystem.usage', 'system.network.io'],
  );
  const used = host.metrics[2].points.find((p) => p.attributes.state === 'used').value;
  assert.ok(used <= 1024 * MIB * 0.98, 'memory never exceeds the machine');
  assert.equal(inventory.attributes['container.id'], ID('b'));
  assert.equal(inventory.attributes['container.hostname'], ID('b').slice(0, 12));
  assert.equal(inventory.metrics[0].name, 'container.cpu.usage.total');
  assert.equal(inventory.metrics[0].points[0].value, 9_000_000_000, 'CPU time in nanoseconds');
});

test('the encoder writes a decodable ExportMetricsServiceRequest', () => {
  const body = encodeMetrics([
    {
      attributes: { 'host.name': 'vps-app' },
      metrics: [
        { name: 'system.cpu.time', unit: 's', kind: 'sum', monotonic: true, points: [{ attributes: { state: 'idle' }, value: 1.5, startNs: 1n, timeNs: 2n }] },
        { name: 'system.cpu.logical.count', kind: 'gauge', points: [{ value: 2, timeNs: 2n }] },
      ],
    },
  ]);
  const [rm] = fields(body)[1];
  const [resource] = fields(rm)[1];
  const [attr] = fields(resource)[1];
  assert.equal(fields(attr)[1][0].toString(), 'host.name');
  const [sm] = fields(rm)[2];
  assert.equal(fields(fields(sm)[1][0])[1][0].toString(), 'karbird-demo-infra-agent');
  const [sum, gauge] = fields(sm)[2].map(fields);
  assert.equal(sum[1][0].toString(), 'system.cpu.time');
  assert.equal(sum[3][0].toString(), 's');
  const sumBody = fields(sum[7][0]);
  assert.equal(sumBody[2][0], 2n, 'cumulative');
  assert.equal(sumBody[3][0], 1n, 'monotonic');
  const pt = fields(sumBody[1][0]);
  assert.equal(pt[4][0].readDoubleLE(), 1.5);
  assert.equal(pt[3][0].readBigUInt64LE(), 2n);
  assert.equal(fields(fields(gauge[5][0])[1][0])[4][0].readDoubleLE(), 2);
});

test('the agent polls the services and exports OTLP metrics with the ingest token', async () => {
  const cap = await captureServer((req, res) => {
    res.writeHead(200, { 'content-type': 'application/json' });
    res.end(req.url === '/__demo/resources' ? JSON.stringify({ pid: 1, cpuMicros: 1234, rssBytes: 50 * MIB, cpuHog: 0 }) : '');
  });
  const agent = await startNode(
    src('infra-agent.mjs'),
    cleanEnv({
      KARBIRD_OTLP_ENDPOINT: cap.url,
      KARBIRD_INGEST_TOKEN: 'demo-token',
      DEMO_INFRA_PLACEMENT: JSON.stringify(placement(cap.url)),
      DEMO_INFRA_INTERVAL_MS: '60000',
    }),
    /exported 2 containers on 1 machines/,
  );
  try {
    const exports = cap.requests.filter((r) => r.url === '/v1/metrics');
    assert.equal(exports.length, 1);
    assert.equal(exports[0].headers.authorization, 'Bearer demo-token');
    assert.equal(exports[0].headers['content-type'], 'application/x-protobuf');
    assert.ok(exports[0].body.includes('system.cpu.time') && exports[0].body.includes(ID('a')));
    assert.equal(cap.requests.filter((r) => r.url === '/__demo/resources').length, 2);
  } finally {
    await agent.stop();
    await cap.close();
  }
});

test('the agent refuses to start without a token', async () => {
  const r = await runNode(src('infra-agent.mjs'), [], cleanEnv({ DEMO_INFRA_PLACEMENT: JSON.stringify(placement()) }));
  assert.equal(r.code, 1);
  assert.match(r.stderr, /KARBIRD_INGEST_TOKEN is required/);
});
