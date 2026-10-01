import { spawn, spawnSync } from "node:child_process";
import { closeSync, existsSync, openSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const outDir = "graphify-out";
const lockPath = join(outDir, ".update.lock");
const logPath = join(outDir, ".update.log");
const staleLockMs = 10 * 60 * 1000;

if (process.argv[2] === "--worker") {
  worker();
} else {
  process.stdin.resume();
  process.stdin.on("data", () => {});
  process.stdin.on("end", hook);
}

function hook() {
  try {
    if (!existsSync(join(outDir, "graph.json"))) return done();
    if (existsSync(lockPath) && Date.now() - statSync(lockPath).mtimeMs < staleLockMs) return done();
    writeFileSync(lockPath, String(process.pid));

    const child = spawn(process.execPath, [fileURLToPath(import.meta.url), "--worker"], {
      detached: true,
      stdio: "ignore",
      windowsHide: true,
    });
    child.unref();
  } catch {
    // fail open: never block the agent because the graph could not be refreshed
  }
  done();
}

function worker() {
  const log = openSync(logPath, "w");
  try {
    spawnSync("graphify", ["update", "."], {
      stdio: ["ignore", log, log],
      windowsHide: true,
      shell: process.platform === "win32",
    });
  } finally {
    closeSync(log);
    rmSync(lockPath, { force: true });
  }
}

function done() {
  process.stdout.write("{}\n");
  process.exit(0);
}
