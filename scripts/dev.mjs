#!/usr/bin/env node
// Build and run karbird backend services (+ optionally the web app); infrastructure is managed separately.
// Run `just bootstrap` once first to create credentials, configuration, and database state.
// `just dev ngrok` also exposes the web app at KARBIRD_PUBLIC_URL through ngrok.
//
// Binaries are copied to .karbird/bin before starting: on Windows a running
// .exe cannot be replaced, and running from target/ would block `cargo build`.
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";
import { devApiEnv, parseDevArgs } from "./dev-args.mjs";
import { assertNotRunning, root, run, supervise } from "./lib.mjs";

let opts;
try {
  opts = parseDevArgs(process.argv.slice(2));
} catch (e) {
  console.error(`[dev] ${e.message}`);
  process.exit(2);
}
if (!existsSync(join(root, ".karbird", "services.env"))) {
  console.error("missing .karbird/services.env — run `just bootstrap` first");
  process.exit(2);
}
assertNotRunning("dev");
const services = ["karbird-ingest", "karbird-processor", "karbird-api", "karbird-executor", "karbird-archiver"];

if (opts.build) {
  await run("cargo", ["build", ...(opts.release ? ["--release"] : []), ...services.flatMap((s) => ["-p", s])]);
}

const exe = (name) => (process.platform === "win32" ? `${name}.exe` : name);
const binDir = join(root, ".karbird", "bin");
mkdirSync(binDir, { recursive: true });
for (const s of services) {
  try {
    copyFileSync(join(root, "target", opts.release ? "release" : "debug", exe(s)), join(binDir, exe(s)));
  } catch (e) {
    if (e?.code !== "EBUSY" && e?.code !== "ETXTBSY") throw e;
    console.error(`${exe(s)} is still running from an earlier \`just dev\`. Stop it first: just stop`);
    process.exit(2);
  }
}
const bin = (name) => join(binDir, exe(name));

const pub = opts.publicUrl;
const apiEnv = devApiEnv(pub);
const procs = [
  { name: "ingest", cmd: bin("karbird-ingest"), args: [] },
  { name: "processor", cmd: bin("karbird-processor"), args: [] },
  { name: "api", cmd: bin("karbird-api"), args: ["serve"], env: apiEnv },
  // The only process allowed to touch the observed system (and Docker).
  { name: "executor", cmd: bin("karbird-executor"), args: [] },
  // Copies closed log windows from ClickHouse into object storage (ADR-0015).
  { name: "archiver", cmd: bin("karbird-archiver"), args: ["run"] },
];
if (opts.web) {
  // Started with node directly (no pnpm/cmd wrapper) so the recorded PID is `next` itself and
  // `just stop` can kill its server child even after the supervisor was force-killed.
  const web = join(root, "apps", "web");
  const next = createRequire(join(web, "package.json")).resolve("next/dist/bin/next");
  procs.push({
    name: "web",
    cmd: process.execPath,
    args: [next, "dev", "--port", "3000", "--hostname", "127.0.0.1"],
    cwd: web,
    env: { KARBIRD_DEV_PUBLIC_URL: pub ?? "" },
  });
}
if (pub) {
  // ngrok reads its own authtoken from its config; everything (pages, /api/v1, /otlp) goes via :3000.
  procs.push({ name: "ngrok", cmd: "ngrok", args: ["http", `--url=${pub}`, "3000", "--log=stdout"] });
  console.log(`[dev] public dev stand: ${pub} (anyone with the URL can reach sign-in and sign-up)`);
}
supervise(procs, "dev");
