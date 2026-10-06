#!/usr/bin/env node
// Start the demo system (demo Postgres, gateway, auth, catalog, orders, inventory, payments,
// notifications, an uninstrumented payment provider stand-in and a load generator)
// sending telemetry to a locally running karbird (`just dev`). Catalog runs as three replicas,
// and a demo infra agent reports the services as containers on two small virtual machines.
// Fault injection is enabled here because this is the demo environment only. A loopback-only
// orchestrator (`scripts/demo-control.mjs`) restarts, scales and releases the services for the
// executor's demo actions.
import { join } from "node:path";
import { CONTROL_PORT, createDemoControl, listenControl } from "./demo-control.mjs";
import { CATALOG_PORTS, PORTS, demoPlacement } from "./demo-placement.mjs";
import { assertNotRunning, compose, loadEnvFile, root, supervise } from "./lib.mjs";

assertNotRunning("demo");

const infra = loadEnvFile(".karbird/infra.env");
const demo = loadEnvFile(".karbird/demo.env");
if (!demo.KARBIRD_INGEST_TOKEN) {
  console.error("missing .karbird/demo.env — run `just bootstrap` first");
  process.exit(1);
}

console.error("[demo] starting demo Postgres…");
await compose(["up", "-d", "--wait", "--wait-timeout", "120", "demo-postgres"], ["--profile", "demo"]);

const cwd = join(root, "examples", "demo-system");
const base = {
  KARBIRD_OTLP_ENDPOINT: demo.KARBIRD_OTLP_ENDPOINT,
  KARBIRD_INGEST_TOKEN: demo.KARBIRD_INGEST_TOKEN,
  KARBIRD_ENVIRONMENT: demo.KARBIRD_ENVIRONMENT,
  KARBIRD_SERVICE_NAMESPACE: "demo",
  KARBIRD_SERVICE_VERSION: "0.1.0",
  KARBIRD_DEMO_FAULTS: "1",
};
const node = process.execPath;
const preload = ["--import", "@karbird/sdk-node/register"];
const pg = {
  CH_DEMO_PG_PORT: infra.CH_DEMO_PG_PORT ?? "15433",
  CH_DEMO_PG_PASSWORD: infra.CH_DEMO_PG_PASSWORD,
};
const placement = demoPlacement();
const where = new Map(placement.flatMap((h) => h.containers.map((c) => [c.name, { host: h.host, id: c.id }])));
const catalogUrls = CATALOG_PORTS.map((p) => `http://127.0.0.1:${p}`).join(",");
const catalogReplicas = CATALOG_PORTS.map((_, i) => `catalog-${i + 1}`);

/** An instrumented demo service, placed in a container on one of the virtual machines. */
const service = (name, script, extra = {}) => {
  const at = where.get(name);
  return {
    name,
    cmd: node,
    args: [...preload, `src/${script}.cjs`],
    cwd,
    env: {
      ...base,
      KARBIRD_SERVICE_NAME: script,
      KARBIRD_SERVICE_INSTANCE_ID: name,
      KARBIRD_HOST_NAME: at?.host ?? "",
      KARBIRD_CONTAINER_ID: at?.id ?? "",
      ...extra,
    },
  };
};

const services = [
  { proc: service("inventory", "inventory", pg), service: "inventory", port: PORTS.inventory },
  { proc: service("payments", "payments"), service: "payments", port: PORTS.payments },
  { proc: service("notifications", "notifications"), service: "notifications", port: PORTS.notifications },
  { proc: service("orders", "orders", pg), service: "orders", port: PORTS.orders },
  { proc: service("auth", "auth"), service: "auth", port: PORTS.auth },
  ...CATALOG_PORTS.map((port, i) => ({
    proc: service(catalogReplicas[i], "catalog", { DEMO_CATALOG_PORT: String(port) }),
    service: "catalog",
    port,
  })),
  { proc: service("gateway", "gateway", { DEMO_CATALOG_URLS: catalogUrls }), service: "gateway", port: PORTS.gateway },
];

const supervisor = supervise([
  // The payment provider stand-in is external to the system: not instrumented.
  { name: "provider", cmd: node, args: ["src/provider.mjs"], cwd, env: {} },
  ...services.map((s) => s.proc),
  {
    name: "infra-agent",
    cmd: node,
    args: ["src/infra-agent.mjs"],
    cwd,
    env: {
      KARBIRD_OTLP_ENDPOINT: demo.KARBIRD_OTLP_ENDPOINT,
      KARBIRD_INGEST_TOKEN: demo.KARBIRD_INGEST_TOKEN,
      DEMO_INFRA_PLACEMENT: JSON.stringify(placement),
    },
  },
  { name: "load", cmd: node, args: ["src/loadgen.mjs"], cwd, env: { DEMO_RPS: process.env.DEMO_RPS ?? "5" } },
], "demo");

const control = createDemoControl({
  supervisor,
  instances: new Map(services.map((s) => [s.proc.name, { service: s.service, port: s.port, proc: s.proc }])),
  scalable: { catalog: { replicas: catalogReplicas, router: "gateway" } },
  releases: { orders: { instance: "orders", versions: ["0.1.0", "0.2.0"], version: base.KARBIRD_SERVICE_VERSION } },
  log: (msg) => console.error(`[demo-control] ${msg}`),
});
const controlPort = Number(process.env.DEMO_CONTROL_PORT ?? CONTROL_PORT);
try {
  await listenControl(control, controlPort);
  console.error(`[demo-control] listening on http://127.0.0.1:${controlPort}`);
} catch (e) {
  console.error(`[demo-control] cannot listen on ${controlPort}: ${e.message}`);
  supervisor.shutdown(1);
}
