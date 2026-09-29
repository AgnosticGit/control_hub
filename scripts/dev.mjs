#!/usr/bin/env node
// Build and run ControlHub backend services (+ optionally the web app)
// against the local infrastructure started by `just bootstrap`.
//
// Binaries are copied to .controlhub/bin before starting: on Windows a running
// .exe cannot be replaced, and running from target/ would block `cargo build`.
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";
import { assertNotRunning, root, run, supervise } from "./lib.mjs";

if (!existsSync(join(root, ".controlhub", "services.env"))) {
  console.error("missing .controlhub/services.env — run `just bootstrap` first");
  process.exit(2);
}
assertNotRunning("dev");
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
  try {
    copyFileSync(join(root, "target", release ? "release" : "debug", exe(s)), join(binDir, exe(s)));
  } catch (e) {
    if (e?.code !== "EBUSY" && e?.code !== "ETXTBSY") throw e;
    console.error(`${exe(s)} is still running from an earlier \`just dev\`. Stop it first: just stop`);
    process.exit(2);
  }
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
  // Started with node directly (no pnpm/cmd wrapper) so the recorded PID is `next` itself and
  // `just stop` can kill its server child even after the supervisor was force-killed.
  const web = join(root, "apps", "web");
  const next = createRequire(join(web, "package.json")).resolve("next/dist/bin/next");
  procs.push({
    name: "web",
    cmd: process.execPath,
    args: [next, "dev", "--port", "3000", "--hostname", "127.0.0.1"],
    cwd: web,
  });
}
supervise(procs, "dev");
