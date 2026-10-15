import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(fileURLToPath(new URL(".", import.meta.url)), "..", "..");
const read = (p) => readFileSync(join(root, p), "utf8");
const config = read("deploy/collector/karbird-collector.yaml");
const compose = read("deploy/collector/compose.yaml");
const postgresConfig = read("deploy/collector/postgres-collector.yaml");

/** Metric names the API reads for the infrastructure layer (`INFRA_METRIC_NAMES`). */
function apiMetricNames() {
  const src = read("crates/karbird-storage/src/ch.rs");
  const block = /INFRA_METRIC_NAMES: &\[&str\] = &\[([\s\S]*?)\];/.exec(src);
  assert.ok(block, "INFRA_METRIC_NAMES not found");
  return [...block[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
}

test("the collector exports exactly the metrics the API reads (utilization gauges are fallbacks)", () => {
  const filter = /filter\/karbird:[\s\S]*?not \(([\s\S]*?)\)\n/.exec(config);
  assert.ok(filter, "filter/karbird not found");
  const exported = [...filter[1].matchAll(/name == "([^"]+)"/g)].map((m) => m[1]).sort();
  const read = apiMetricNames().filter((n) => !n.endsWith(".utilization")).sort();
  assert.deepEqual(exported, read);
});

test("docker_stats enables container network counters consumed by the infrastructure API", () => {
  const dockerStats = /docker_stats:\n([\s\S]*?)\nprocessors:/.exec(config);
  assert.ok(dockerStats, "docker_stats receiver not found");
  for (const metric of ["container.network.io.usage.rx_bytes", "container.network.io.usage.tx_bytes"]) {
    assert.match(dockerStats[1], new RegExp(`${metric.replaceAll(".", "\\.")}:\\s+enabled: true`));
  }
});

test("the collector reports host.name from KARBIRD_HOST_NAME and sends the ingest token", () => {
  assert.match(config, /key: host\.name\s+value: \$\{env:KARBIRD_HOST_NAME\}/);
  assert.match(config, /Authorization: Bearer \$\{env:KARBIRD_INGEST_TOKEN\}/);
  assert.match(config, /root_path: \/hostfs/);
  assert.match(config, /receivers: \[hostmetrics, docker_stats\]/);
});

test("the collector keeps the image name and maps only the Compose service label", () => {
  assert.doesNotMatch(config, /key: container\.image\.name\s+action: delete/, "image name places databases by image");
  const labels = /container_labels_to_metric_labels:\n((?:\s{6}\S.*\n)+)/.exec(config);
  assert.ok(labels, "container_labels_to_metric_labels not found");
  assert.deepEqual(labels[1].trim().split("\n").map((l) => l.trim()), ["com.docker.compose.service: docker.compose.service"]);
  assert.doesNotMatch(config, /env_vars_to_metric_labels/, "no container environment");
});

test("the collector image is pinned and host mounts are read-only", () => {
  const image = /image: (\S+)/.exec(compose)?.[1] ?? "";
  assert.match(image, /^otel\/opentelemetry-collector-contrib:\d+\.\d+\.\d+$/);
  assert.doesNotMatch(compose, /:latest\b/);
  for (const mount of ["/:/hostfs:ro", "/var/run/docker.sock:/var/run/docker.sock:ro"]) {
    assert.ok(compose.includes(mount), mount);
  }
  assert.doesNotMatch(compose, /KARBIRD_INGEST_TOKEN: [^$]/, "no token in the file");
  assert.match(compose, /name: karbird-collector/);
  assert.match(compose, /services:\s+collector:/);
});

test("optional PostgreSQL collector uses read-only statistics and the scoped metrics token", () => {
  assert.match(postgresConfig, /postgresql:\s*\n\s*endpoint: \$\{env:KARBIRD_POSTGRES_ENDPOINT\}/);
  assert.match(postgresConfig, /username: \$\{env:KARBIRD_POSTGRES_USER\}/);
  assert.match(postgresConfig, /password: \$\{env:KARBIRD_POSTGRES_PASSWORD\}/);
  assert.match(postgresConfig, /receivers: \[postgresql\]/);
  assert.match(postgresConfig, /Authorization: Bearer \$\{env:KARBIRD_INGEST_TOKEN\}/);
  assert.doesNotMatch(postgresConfig, /db\.server\.query_sample|db\.server\.top_query/);
  assert.match(compose, /profiles: \["postgres"\]/);
});
