import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(fileURLToPath(new URL(".", import.meta.url)), "..", "..");
const read = (p) => readFileSync(join(root, p), "utf8");
const config = read("deploy/agent/karbird-agent.yaml");
const compose = read("deploy/agent/compose.yaml");

/** Metric names the API reads for the infrastructure layer (`INFRA_METRIC_NAMES`). */
function apiMetricNames() {
  const src = read("crates/karbird-storage/src/ch.rs");
  const block = /INFRA_METRIC_NAMES: &\[&str\] = &\[([\s\S]*?)\];/.exec(src);
  assert.ok(block, "INFRA_METRIC_NAMES not found");
  return [...block[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
}

test("the agent exports exactly the metrics the API reads (utilization gauges are fallbacks)", () => {
  const filter = /filter\/karbird:[\s\S]*?not \(([\s\S]*?)\)\n/.exec(config);
  assert.ok(filter, "filter/karbird not found");
  const exported = [...filter[1].matchAll(/name == "([^"]+)"/g)].map((m) => m[1]).sort();
  const read = apiMetricNames().filter((n) => !n.endsWith(".utilization")).sort();
  assert.deepEqual(exported, read);
});

test("the agent reports host.name from KARBIRD_HOST_NAME and sends the ingest token", () => {
  assert.match(config, /key: host\.name\s+value: \$\{env:KARBIRD_HOST_NAME\}/);
  assert.match(config, /Authorization: Bearer \$\{env:KARBIRD_INGEST_TOKEN\}/);
  assert.match(config, /root_path: \/hostfs/);
  assert.match(config, /receivers: \[hostmetrics, docker_stats\]/);
});

test("the agent image is pinned and host mounts are read-only", () => {
  const image = /image: (\S+)/.exec(compose)?.[1] ?? "";
  assert.match(image, /^otel\/opentelemetry-collector-contrib:\d+\.\d+\.\d+$/);
  assert.doesNotMatch(compose, /:latest\b/);
  for (const mount of ["/:/hostfs:ro", "/var/run/docker.sock:/var/run/docker.sock:ro"]) {
    assert.ok(compose.includes(mount), mount);
  }
  assert.doesNotMatch(compose, /KARBIRD_INGEST_TOKEN: [^$]/, "no token in the file");
});
