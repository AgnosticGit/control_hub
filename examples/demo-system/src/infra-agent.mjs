// Demo stand-in for the Karbird agent (OpenTelemetry Collector with hostmetrics + docker_stats).
// The demo services all run on this computer, so the agent pretends they are placed on small
// virtual machines (DEMO_INFRA_PLACEMENT) and reports, under the same metric names as the
// real agent:
// - per container: the real CPU time and RSS of the service process (`/__demo/resources`);
//   the demo database container reports a steady synthetic load, image and Compose service;
// - per machine: CPU time = the sum of its containers plus a small base load, capped by the
//   machine's cores; memory = base + container RSS; disk and network are synthetic.
// DEMO ONLY.

import { pathToFileURL } from 'node:url';
import { encodeMetrics } from './otlp-metrics.mjs';

const MIB = 1024 * 1024;
const BASE_CPU_SHARE = 0.03;
const BASE_MEMORY = 220 * MIB;
const DISK_USED = 9.4 * 1024 * MIB;
const NET_BYTES_PER_CPU_SECOND = 4 * MIB;

/**
 * A container is measured from a demo service (`url`) or, for stand-ins like the demo
 * database, reports a steady synthetic load (`synthetic`).
 * @typedef {{ cores: number, rssMb: number }} SyntheticLoad
 * @typedef {{ name: string, id: string, url?: string, synthetic?: SyntheticLoad, image?: string, service?: string }} ContainerSpec
 * @typedef {{ host: string, cores: number, memoryMb: number, diskGb: number, containers: ContainerSpec[] }} HostSpec
 */

const positive = (v) => Number.isFinite(v) && v > 0;

/** Validates DEMO_INFRA_PLACEMENT. */
export function parsePlacement(text) {
  const hosts = JSON.parse(text);
  if (!Array.isArray(hosts) || hosts.length === 0) throw new Error('placement must be a non-empty array');
  for (const h of hosts) {
    if (typeof h.host !== 'string' || !h.host) throw new Error('every machine needs a host name');
    for (const k of ['cores', 'memoryMb', 'diskGb']) {
      if (!positive(h[k])) throw new Error(`${h.host}: ${k} must be positive`);
    }
    if (!Array.isArray(h.containers)) throw new Error(`${h.host}: containers must be an array`);
    for (const c of h.containers) {
      if (!c.name || !/^[0-9a-f]{64}$/.test(c.id ?? '')) throw new Error(`${h.host}: container needs name and 64-hex id`);
      const synthetic = c.synthetic !== undefined && positive(c.synthetic?.cores) && positive(c.synthetic?.rssMb);
      if (!c.url === !synthetic) throw new Error(`${h.host}: container ${c.name} needs either a url or a synthetic load`);
      for (const k of ['image', 'service']) {
        if (c[k] !== undefined && (typeof c[k] !== 'string' || !c[k])) throw new Error(`${h.host}: ${c.name}.${k} must be a string`);
      }
    }
  }
  return hosts;
}

/** Cumulative CPU and RSS of a synthetic container `elapsedSeconds` after the agent started. */
export function syntheticSample(load, elapsedSeconds) {
  return { cpuMicros: Math.round(load.cores * Math.max(0, elapsedSeconds) * 1e6), rssBytes: load.rssMb * MIB };
}

/** Cumulative counters of one machine; `step` adds one sampling interval. */
export function createHostState(spec) {
  return { spec, busySeconds: 0, idleSeconds: 0, rxBytes: 0, txBytes: 0 };
}

/**
 * Advances a machine by `dtSeconds` given the CPU seconds its containers used in that time.
 * Busy time never exceeds the machine's cores; a hog shows up as a full machine.
 */
export function stepHost(state, dtSeconds, containerCpuSeconds) {
  const capacity = state.spec.cores * dtSeconds;
  const busy = Math.min(capacity, containerCpuSeconds + BASE_CPU_SHARE * capacity);
  state.busySeconds += busy;
  state.idleSeconds += capacity - busy;
  const net = busy * NET_BYTES_PER_CPU_SECOND;
  state.rxBytes += net;
  state.txBytes += net * 0.6;
  return busy / capacity;
}

/** OTLP resources for one sampling round. `samples` maps container id to `{ cpuMicros, rssBytes }`. */
export function buildResources(states, samples, startNs, timeNs) {
  const resources = [];
  for (const state of states) {
    const { spec } = state;
    const cores = spec.cores;
    const memTotal = spec.memoryMb * MIB;
    let rss = 0;
    for (const c of spec.containers) rss += samples.get(c.id)?.rssBytes ?? 0;
    const memUsed = Math.min(memTotal * 0.98, BASE_MEMORY + rss);
    const diskTotal = spec.diskGb * 1024 * MIB;
    const sum = (name, unit, monotonic, points) => ({ name, unit, kind: 'sum', monotonic, points });
    const gauge = (name, unit, points) => ({ name, unit, kind: 'gauge', points });
    const pt = (value, attributes = {}, cumulative = true) => ({ value, attributes, timeNs, ...(cumulative ? { startNs } : {}) });
    const cpuPoints = [];
    for (let i = 0; i < cores; i++) {
      cpuPoints.push(pt(state.busySeconds / cores, { cpu: `cpu${i}`, state: 'user' }));
      cpuPoints.push(pt(state.idleSeconds / cores, { cpu: `cpu${i}`, state: 'idle' }));
    }
    resources.push({
      attributes: { 'host.name': spec.host, 'host.id': spec.host, 'os.type': 'linux' },
      metrics: [
        sum('system.cpu.time', 's', true, cpuPoints),
        gauge('system.cpu.logical.count', '{cpu}', [pt(cores, {}, false)]),
        sum('system.memory.usage', 'By', false, [pt(memUsed, { state: 'used' }), pt(memTotal - memUsed, { state: 'free' })]),
        gauge('system.memory.limit', 'By', [pt(memTotal, {}, false)]),
        sum('system.filesystem.usage', 'By', false, [
          pt(DISK_USED, { device: '/dev/vda1', mountpoint: '/', state: 'used' }),
          pt(diskTotal - DISK_USED, { device: '/dev/vda1', mountpoint: '/', state: 'free' }),
        ]),
        sum('system.network.io', 'By', true, [
          pt(state.rxBytes, { device: 'eth0', direction: 'receive' }),
          pt(state.txBytes, { device: 'eth0', direction: 'transmit' }),
        ]),
      ],
    });
    for (const c of spec.containers) {
      const s = samples.get(c.id);
      if (!s) continue;
      resources.push({
        attributes: {
          'host.name': spec.host,
          'host.id': spec.host,
          'container.id': c.id,
          'container.name': c.name,
          'container.hostname': c.id.slice(0, 12),
          ...(c.image ? { 'container.image.name': c.image } : {}),
          ...(c.service ? { 'docker.compose.service': c.service } : {}),
        },
        metrics: [
          sum('container.cpu.usage.total', 'ns', true, [pt(s.cpuMicros * 1000)]),
          gauge('container.memory.usage.total', 'By', [pt(s.rssBytes, {}, false)]),
          gauge('container.memory.usage.limit', 'By', [pt(memTotal, {}, false)]),
        ],
      });
    }
  }
  return resources;
}

async function sample(url) {
  const res = await fetch(`${url}/__demo/resources`, { signal: AbortSignal.timeout(2000) });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  const body = await res.json();
  if (!Number.isFinite(body.cpuMicros) || !Number.isFinite(body.rssBytes)) throw new Error('bad resources reply');
  return { cpuMicros: body.cpuMicros, rssBytes: body.rssBytes };
}

const nowNs = () => BigInt(Date.now()) * 1_000_000n;

async function main() {
  const endpoint = (process.env.KARBIRD_OTLP_ENDPOINT ?? 'http://127.0.0.1:4318').replace(/\/$/, '');
  const token = process.env.KARBIRD_INGEST_TOKEN;
  if (!token) {
    console.error('infra-agent: KARBIRD_INGEST_TOKEN is required');
    process.exit(1);
  }
  const placement = parsePlacement(process.env.DEMO_INFRA_PLACEMENT ?? '[]');
  const intervalMs = Number(process.env.DEMO_INFRA_INTERVAL_MS ?? 10_000);
  const states = placement.map(createHostState);
  const startNs = nowNs();
  const startedAt = Date.now();
  const lastCpu = new Map();
  let last = Date.now();

  const tick = async () => {
    const now = Date.now();
    const dt = Math.max(0.001, (now - last) / 1000);
    last = now;
    const samples = new Map();
    await Promise.all(
      placement.flatMap((h) =>
        h.containers.map(async (c) => {
          try {
            samples.set(c.id, c.synthetic ? syntheticSample(c.synthetic, (now - startedAt) / 1000) : await sample(c.url));
          } catch {
            // The service may still be starting; it is simply missing from this round.
          }
        }),
      ),
    );
    for (const state of states) {
      let cpuSeconds = 0;
      for (const c of state.spec.containers) {
        const s = samples.get(c.id);
        if (!s) continue;
        const prev = lastCpu.get(c.id);
        if (prev !== undefined && s.cpuMicros >= prev) cpuSeconds += (s.cpuMicros - prev) / 1e6;
        lastCpu.set(c.id, s.cpuMicros);
      }
      stepHost(state, dt, cpuSeconds);
    }
    const body = encodeMetrics(buildResources(states, samples, startNs, nowNs()));
    try {
      const res = await fetch(`${endpoint}/v1/metrics`, {
        method: 'POST',
        headers: { authorization: `Bearer ${token}`, 'content-type': 'application/x-protobuf' },
        body,
        signal: AbortSignal.timeout(5000),
      });
      await res.arrayBuffer();
      if (!res.ok) console.error(`infra-agent: export failed (HTTP ${res.status})`);
      else console.log(`infra-agent: exported ${samples.size} containers on ${states.length} machines`);
    } catch (e) {
      console.error(`infra-agent: export failed: ${e.message}`);
    }
  };

  console.log(`infra-agent: ${states.length} machines, every ${intervalMs} ms`);
  await tick();
  setInterval(() => void tick(), intervalMs);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((e) => {
    console.error(`infra-agent: ${e.message}`);
    process.exit(1);
  });
}