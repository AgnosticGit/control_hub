// Demo orchestrator: plays the container platform of `just demo`. DEMO ONLY.
// It restarts service processes, scales the catalog replicas behind the gateway and rolls
// releases (a release is the service restarted with another KARBIRD_SERVICE_VERSION), so the
// executor's demo adapter has real effects to apply and verify (ADR-0020).
// It listens on loopback only and accepts only names and versions it was configured with.
import { createServer } from "node:http";

export const CONTROL_PORT = 7090;
const BODY_LIMIT = 4096;

export class HttpError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/**
 * @typedef {{ name: string, cmd: string, args: string[], cwd?: string, env?: Record<string, string> }} Proc
 * @typedef {{ service: string, port: number, proc: Proc }} Instance
 * @typedef {{ start: (p: Proc) => unknown, stop: (name: string) => Promise<unknown>, running: (name: string) => boolean, pid: (name: string) => number | null }} Supervisor
 * @param {{
 *   supervisor: Supervisor,
 *   instances: Map<string, Instance>,
 *   scalable?: Record<string, { replicas: string[], router: string }>,
 *   releases?: Record<string, { instance: string, versions: string[], version: string }>,
 *   healthTimeoutMs?: number,
 *   pollMs?: number,
 *   log?: (msg: string) => void,
 *   onRelease?: (change: { service: string, from_version: string, to_version: string, kind: 'deploy' | 'rollback' }) => Promise<void>,
 *   setIncidentCycleEnabled?: (enabled: boolean) => void,
 * }} opts
 */
export function createDemoControl({ supervisor, instances, scalable = {}, releases = {}, healthTimeoutMs = 30_000, pollMs = 200, log = () => {}, onRelease = async () => {}, setIncidentCycleEnabled = () => {} }) {
  const overrides = new Map();
  const releaseState = Object.fromEntries(
    Object.entries(releases).map(([service, r]) => [service, { ...r, history: [] }]),
  );
  for (const r of Object.values(releaseState)) overrides.set(r.instance, { KARBIRD_SERVICE_VERSION: r.version });

  let queue = Promise.resolve();
  /** Operations change process state: one at a time. */
  const serial = (fn) => {
    const run = queue.then(fn, fn);
    queue = run.catch(() => {});
    return run;
  };

  const instance = (name) => {
    const def = instances.get(name);
    if (!def) throw new HttpError(404, `unknown instance ${name}`);
    return def;
  };
  const urlOf = (name) => `http://127.0.0.1:${instance(name).port}`;
  const procOf = (name) => {
    const def = instance(name);
    return { ...def.proc, env: { ...def.proc.env, ...overrides.get(name) } };
  };
  const replicaSet = (service) => {
    const s = scalable[service];
    if (!s) throw new HttpError(404, `${service} cannot be scaled`);
    return s;
  };
  const replicaOf = (name) => Object.keys(scalable).find((s) => scalable[s].replicas.includes(name));
  const runningReplicas = (service) => replicaSet(service).replicas.filter((n) => supervisor.running(n));

  async function waitHealthy(name) {
    const deadline = Date.now() + healthTimeoutMs;
    for (;;) {
      try {
        const res = await fetch(`${urlOf(name)}/health`, { signal: AbortSignal.timeout(1000) });
        await res.arrayBuffer();
        if (res.ok) return;
      } catch {
        // not listening yet
      }
      if (!supervisor.running(name)) throw new HttpError(500, `${name} exited while starting`);
      if (Date.now() >= deadline) throw new HttpError(504, `${name} did not become healthy within ${healthTimeoutMs} ms`);
      await sleep(pollMs);
    }
  }

  async function routerCall(service, method, body) {
    const router = replicaSet(service).router;
    const res = await fetch(`${urlOf(router)}/__demo/upstreams`, {
      method,
      headers: body ? { "content-type": "application/json" } : {},
      body: body ? JSON.stringify(body) : undefined,
      signal: AbortSignal.timeout(3000),
    });
    const reply = await res.json().catch(() => null);
    if (!res.ok || !Array.isArray(reply?.[service])) throw new HttpError(502, `${router} rejected the ${service} upstream list (${res.status})`);
    return reply[service];
  }

  /** Points the router at exactly `names` (all running replicas by default). */
  const route = (service, names = runningReplicas(service)) => routerCall(service, "POST", { [service]: names.map(urlOf) });

  async function readUpstreams(service) {
    try {
      return await routerCall(service, "GET");
    } catch {
      return null;
    }
  }

  async function startInstance(name) {
    supervisor.start(procOf(name));
    await waitHealthy(name);
    // A restarted router starts with its configured list: give it the current replicas again.
    for (const [service, s] of Object.entries(scalable)) {
      if (s.router === name) await route(service);
    }
  }

  async function restartUnlocked(name) {
    const def = instance(name);
    if (!supervisor.running(name)) throw new HttpError(409, `${name} is not running`);
    const service = replicaOf(name);
    const previousPid = supervisor.pid(name);
    const startedAt = Date.now();
    // Drain a replica first so the router sends no requests to a stopping process.
    if (service) {
      const others = runningReplicas(service).filter((n) => n !== name);
      if (others.length > 0) await route(service, others);
    }
    await supervisor.stop(name);
    await startInstance(name);
    if (service) await route(service);
    log(`restarted ${name}`);
    return { instance: name, service: def.service, previous_pid: previousPid, pid: supervisor.pid(name), healthy_after_ms: Date.now() - startedAt };
  }

  async function replicaState(service) {
    const s = replicaSet(service);
    const running = runningReplicas(service);
    return { service, replicas: running.length, max: s.replicas.length, running, upstreams: await readUpstreams(service) };
  }

  async function scaleUnlocked(service, replicas) {
    const s = replicaSet(service);
    if (!Number.isInteger(replicas) || replicas < 1 || replicas > s.replicas.length) {
      throw new HttpError(400, `replicas must be an integer in [1, ${s.replicas.length}]`);
    }
    const running = runningReplicas(service);
    if (replicas > running.length) {
      const missing = s.replicas.filter((n) => !supervisor.running(n)).slice(0, replicas - running.length);
      for (const name of missing) await startInstance(name);
      await route(service);
    } else if (replicas < running.length) {
      const keep = running.slice(0, replicas);
      await route(service, keep);
      for (const name of running.slice(replicas)) await supervisor.stop(name);
    }
    log(`scaled ${service} to ${replicas}`);
    return replicaState(service);
  }

  const release = (service) => {
    const r = releaseState[service];
    if (!r) throw new HttpError(404, `${service} has no releases`);
    return r;
  };
  const releaseView = (service) => {
    const r = release(service);
    return { service, version: r.version, previous: r.history.at(-1) ?? null, versions: r.versions };
  };

  async function switchVersion(service, version) {
    const r = release(service);
    if (!supervisor.running(r.instance)) throw new HttpError(409, `${r.instance} is not running`);
    const before = overrides.get(r.instance);
    overrides.set(r.instance, { ...before, KARBIRD_SERVICE_VERSION: version });
    try {
      const restarted = await restartUnlocked(r.instance);
      r.version = version;
      return restarted;
    } catch (e) {
      overrides.set(r.instance, before);
      throw e;
    }
  }

  async function deployUnlocked(service, version) {
    const r = release(service);
    if (typeof version !== "string" || !r.versions.includes(version)) {
      throw new HttpError(400, `version must be one of ${r.versions.join(", ")}`);
    }
    if (version === r.version) throw new HttpError(409, `${service} already runs ${version}`);
    const from = r.version;
    const restarted = await switchVersion(service, version);
    r.history.push(from);
    log(`released ${service} ${version}`);
    try {
      await onRelease({ service, from_version: from, to_version: version, kind: 'deploy' });
    } catch (error) {
      log(`release event failed for ${service}: ${error instanceof Error ? error.message : String(error)}`);
    }
    return { ...restarted, service, from_version: from, to_version: version };
  }

  async function rollbackUnlocked(service) {
    const r = release(service);
    const to = r.history.at(-1);
    if (to === undefined) throw new HttpError(409, `${service} has no earlier release to roll back to`);
    const from = r.version;
    const restarted = await switchVersion(service, to);
    r.history.pop();
    log(`rolled back ${service} ${from} -> ${to}`);
    try {
      await onRelease({ service, from_version: from, to_version: to, kind: 'rollback' });
    } catch (error) {
      log(`release event failed for ${service}: ${error instanceof Error ? error.message : String(error)}`);
    }
    return { ...restarted, service, from_version: from, to_version: to };
  }

  return {
    instances: () =>
      [...instances.entries()].map(([name, def]) => ({
        name,
        service: def.service,
        port: def.port,
        running: supervisor.running(name),
        pid: supervisor.pid(name),
        version: overrides.get(name)?.KARBIRD_SERVICE_VERSION ?? def.proc.env?.KARBIRD_SERVICE_VERSION ?? null,
      })),
    restart: (name) => serial(() => restartUnlocked(name)),
    replicas: (service) => replicaState(service),
    scale: (service, replicas) => serial(() => scaleUnlocked(service, replicas)),
    release: (service) => releaseView(service),
    deploy: (service, version) => serial(() => deployUnlocked(service, version)),
    rollback: (service) => serial(() => rollbackUnlocked(service)),
    setIncidentsEnabled: (enabled) => {
      if (typeof enabled !== 'boolean') throw new HttpError(400, 'enabled must be a boolean');
      setIncidentCycleEnabled(enabled);
      return { enabled };
    },
  };
}

const LOOPBACK = new Set(["127.0.0.1", "::1", "::ffff:127.0.0.1"]);

function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let size = 0;
    req.on("data", (c) => {
      size += c.length;
      if (size <= BODY_LIMIT) chunks.push(c);
    });
    req.on("end", () => {
      if (size > BODY_LIMIT) return reject(new HttpError(413, "body too large"));
      if (chunks.length === 0) return resolve({});
      try {
        const v = JSON.parse(Buffer.concat(chunks).toString("utf8"));
        resolve(v !== null && typeof v === "object" && !Array.isArray(v) ? v : {});
      } catch {
        reject(new HttpError(400, "invalid JSON"));
      }
    });
    req.on("error", reject);
  });
}

/** HTTP front of the orchestrator; every route answers JSON. */
export function controlHandler(control) {
  const routes = [
    ["GET", /^\/instances$/, () => ({ instances: control.instances() })],
    ["POST", /^\/instances\/([a-z0-9-]{1,32})\/restart$/, (m) => control.restart(m[1])],
    ["GET", /^\/replicas\/([a-z0-9-]{1,32})$/, (m) => control.replicas(m[1])],
    ["POST", /^\/replicas\/([a-z0-9-]{1,32})$/, (m, body) => control.scale(m[1], body.replicas)],
    ["GET", /^\/releases\/([a-z0-9-]{1,32})$/, (m) => control.release(m[1])],
    ["POST", /^\/releases\/([a-z0-9-]{1,32})$/, (m, body) => control.deploy(m[1], body.version)],
    ["POST", /^\/releases\/([a-z0-9-]{1,32})\/rollback$/, (m) => control.rollback(m[1])],
    ["POST", /^\/incident-cycle$/, (_m, body) => control.setIncidentsEnabled(body.enabled)],
  ];
  return async (req, res) => {
    const send = (status, body) => {
      const text = JSON.stringify(body);
      res.writeHead(status, { "content-type": "application/json", "content-length": Buffer.byteLength(text) });
      res.end(text);
    };
    if (!LOOPBACK.has(req.socket.remoteAddress ?? "")) return send(404, { error: "not found" });
    const path = (req.url ?? "/").split("?")[0];
    const matching = routes.filter(([, re]) => re.test(path));
    if (matching.length === 0) return send(404, { error: "not found" });
    const hit = matching.find(([method]) => method === req.method);
    if (!hit) return send(405, { error: "method not allowed" });
    try {
      const body = req.method === "POST" ? await readBody(req) : {};
      send(200, await hit[2](path.match(hit[1]), body));
    } catch (e) {
      send(e instanceof HttpError ? e.status : 500, { error: e instanceof HttpError ? e.message : "internal error" });
    }
  };
}

/** Starts the orchestrator on 127.0.0.1:`port`; resolves with the listening server. */
export function listenControl(control, port = CONTROL_PORT) {
  const server = createServer(controlHandler(control));
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", () => resolve(server));
  });
}
