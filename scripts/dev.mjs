#!/usr/bin/env node
// Build and run ControlHub backend services (+ optionally the web app)
// against the local infrastructure started by `just bootstrap`.
//
// Binaries are copied to .controlhub/bin before starting: on Windows a running
// .exe cannot be replaced, and running from target/ would block `cargo build`.
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { root, run, supervise } from "./lib.mjs";

if (!existsSync(join(root, ".controlhub", "services.env"))) {
  console.error("missing .controlhub/services.env — run `just bootstrap` first");
  process.exit(2);
}
const withWeb = !process.argv.includes("--no-web");
const release = process.argv.includes("--release");
const skipBuild = process.argv.includes("--no-build");
const services = ["controlhub-ingest", "controlhub-processor", "controlhub-api", "controlhub-executor"];

if (!skipBuild) {
  await run("cargo", ["build", ...(release ? ["--release"] : []), ...services.flatMap((s) => ["-p", s])]);
}

const exe = (name) => (process.platform === "win32" ? `${name}.exe` : name);
const binDir = join(root, ".controlhub", "bin");
mkdirSync(binDir, { recursive: true });
for (const s of services) {
  copyFileSync(join(root, "target", release ? "release" : "debug", exe(s)), join(binDir, exe(s)));
}
const bin = (name) => join(binDir, exe(name));

const procs = [
  { name: "ingest", cmd: bin("controlhub-ingest"), args: [] },
  { name: "processor", cmd: bin("controlhub-processor"), args: [] },
  { name: "api", cmd: bin("controlhub-api"), args: ["serve"] },
  // The only process allowed to touch the observed system (and Docker).
  { name: "executor", cmd: bin("controlhub-executor"), args: [] },
];
if (withWeb) {
  procs.push({ name: "web", cmd: "pnpm", args: ["--filter", "@controlhub/web", "dev"] });
}
supervise(procs);
