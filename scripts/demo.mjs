#!/usr/bin/env node
// Start the demo system (demo Postgres, orders, gateway, load generator)
// sending telemetry to a locally running ControlHub (`just dev`).
// Fault injection is enabled here because this is the demo environment only.
import { join } from "node:path";
import { compose, loadEnvFile, root, supervise } from "./lib.mjs";

const infra = loadEnvFile(".controlhub/infra.env");
const demo = loadEnvFile(".controlhub/demo.env");
if (!demo.CONTROLHUB_INGEST_TOKEN) {
  console.error("missing .controlhub/demo.env — run `just bootstrap` first");
  process.exit(1);
}

console.error("[demo] starting demo Postgres…");
await compose(["up", "-d", "--wait", "--wait-timeout", "120", "demo-postgres"], ["--profile", "demo"]);

const cwd = join(root, "examples", "demo-system");
const base = {
  CONTROLHUB_OTLP_ENDPOINT: demo.CONTROLHUB_OTLP_ENDPOINT,
  CONTROLHUB_INGEST_TOKEN: demo.CONTROLHUB_INGEST_TOKEN,
  CONTROLHUB_ENVIRONMENT: demo.CONTROLHUB_ENVIRONMENT,
  CONTROLHUB_SERVICE_NAMESPACE: "demo",
  CONTROLHUB_SERVICE_VERSION: "0.1.0",
  CONTROLHUB_DEMO_FAULTS: "1",
};
const node = process.execPath;
const preload = ["--import", "@controlhub/sdk-node/register"];

supervise([
  {
    name: "orders",
    cmd: node,
    args: [...preload, "src/orders.cjs"],
    cwd,
    env: {
      ...base,
      CONTROLHUB_SERVICE_NAME: "orders",
      CH_DEMO_PG_PORT: infra.CH_DEMO_PG_PORT ?? "15433",
      CH_DEMO_PG_PASSWORD: infra.CH_DEMO_PG_PASSWORD,
    },
  },
  { name: "gateway", cmd: node, args: [...preload, "src/gateway.cjs"], cwd, env: { ...base, CONTROLHUB_SERVICE_NAME: "gateway" } },
  { name: "load", cmd: node, args: ["src/loadgen.mjs"], cwd, env: { DEMO_RPS: process.env.DEMO_RPS ?? "5" } },
]);
