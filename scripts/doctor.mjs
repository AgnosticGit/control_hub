#!/usr/bin/env node
// Check local prerequisites. Prints one line per check; exits 1 if a required
// tool is missing. Never installs anything.
import { execFileSync } from "node:child_process";
import { createServer } from "node:net";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { needsShell, root } from "./lib.mjs";

const results = [];
function version(cmd, args = ["--version"]) {
  try {
    const out = execFileSync(needsShell(cmd) ? `${cmd} ${args.join(" ")}` : cmd, needsShell(cmd) ? [] : args, {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      shell: needsShell(cmd),
      timeout: 20_000,
    });
    return out.trim().split(/\r?\n/)[0];
  } catch {
    return null;
  }
}
function check(name, ok, detail, required = true) {
  results.push({ name, ok, detail, required });
}

const toolchain = readFileSync(join(root, "rust-toolchain.toml"), "utf8").match(/channel\s*=\s*"([^"]+)"/)?.[1];
const rustc = version("rustc");
check("rustc", Boolean(rustc), `${rustc ?? "missing"} (pinned: ${toolchain})`);
check("cargo", Boolean(version("cargo")), version("cargo") ?? "missing");
const targets = version("rustup", ["target", "list", "--installed"]) === null ? "" : execFileSync("rustup", ["target", "list", "--installed"], { encoding: "utf8" });
check("wasm32 target", targets.includes("wasm32-unknown-unknown"), targets.includes("wasm32-unknown-unknown") ? "installed" : "rustup target add wasm32-unknown-unknown");
const bindgen = version("wasm-bindgen");
check("wasm-bindgen CLI 0.2.129", bindgen?.includes("0.2.129") ?? false, bindgen ?? "cargo install wasm-bindgen-cli --version 0.2.129 --locked");
const node = process.versions.node;
check("node >= 22.12", Number(node.split(".")[0]) >= 22, node);
check("pnpm 10", version("pnpm")?.startsWith("10") ?? false, version("pnpm") ?? "npm i -g pnpm@10 (or corepack enable)");
check("just", Boolean(version("just")), version("just") ?? "cargo install just --locked");
const docker = version("docker");
check("docker", Boolean(docker), docker ?? "missing");
const compose = version("docker", ["compose", "version"]);
check("docker compose v2", Boolean(compose), compose ?? "missing");
let daemon = false;
try {
  execFileSync("docker", ["info", "--format", "{{.ServerVersion}}"], { stdio: "ignore", timeout: 20_000 });
  daemon = true;
} catch {
  daemon = false;
}
check("docker daemon running", daemon, daemon ? "ok" : "start Docker Desktop / dockerd");

const infraEnv = join(root, ".controlhub", "infra.env");
const ports = { pg: 15432, clickhouse: 18123, kafka: 19092, demoPg: 15433, s3: 19000 };
if (existsSync(infraEnv)) {
  const env = Object.fromEntries(readFileSync(infraEnv, "utf8").split(/\r?\n/).filter((l) => l.includes("=")).map((l) => l.split("=", 2)));
  ports.pg = Number(env.CH_PG_PORT ?? ports.pg);
  ports.clickhouse = Number(env.CH_CLICKHOUSE_HTTP_PORT ?? ports.clickhouse);
  ports.kafka = Number(env.CH_KAFKA_PORT ?? ports.kafka);
  ports.s3 = Number(env.CH_S3_PORT ?? ports.s3);
}
const servicePorts = { ingestHttp: 4318, ingestGrpc: 4317, api: 8080, web: 3000, gateway: 7001, orders: 7002, inventory: 7003, payments: 7004, notifications: 7005, catalog: 7006, auth: 7007, provider: 7010 };
function portFree(port) {
  return new Promise((resolve) => {
    const s = createServer();
    s.once("error", () => resolve(false));
    s.listen(port, "127.0.0.1", () => s.close(() => resolve(true)));
  });
}
for (const [name, port] of Object.entries({ ...ports, ...servicePorts })) {
  const free = await portFree(port);
  check(`port ${port} (${name})`, free, free ? "free" : "in use (fine if ControlHub is already running)", false);
}
check("bootstrap done", existsSync(infraEnv), existsSync(infraEnv) ? ".controlhub/ present" : "run just bootstrap", false);
const wasmBuilt = existsSync(join(root, "apps/web/src/generated/wasm-manifest.ts"));
check("wasm built", wasmBuilt, wasmBuilt ? "apps/web/src/generated/wasm-manifest.ts" : "run just wasm", false);

let failed = false;
for (const r of results) {
  const mark = r.ok ? "ok  " : r.required ? "FAIL" : "warn";
  if (!r.ok && r.required) failed = true;
  console.log(`${mark}  ${r.name.padEnd(28)} ${r.detail}`);
}
if (process.argv.includes("--json")) console.log(JSON.stringify(results));
process.exit(failed ? 1 : 0);
