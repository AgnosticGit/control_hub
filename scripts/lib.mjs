import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const root = join(dirname(fileURLToPath(import.meta.url)), "..");

export function parseEnv(text) {
  const out = {};
  for (const line of text.split(/\r?\n/)) {
    const t = line.trim();
    if (!t || t.startsWith("#")) continue;
    const i = t.indexOf("=");
    if (i > 0) out[t.slice(0, i)] = t.slice(i + 1);
  }
  return out;
}

/**
 * setup → body → teardown, where teardown always runs (also after a failed setup or body).
 * Resolves with the body's exit code; a failed setup or teardown turns it into 1.
 */
export async function withFixtures({ setup, body, teardown, log = console.error }) {
  let code = 1;
  try {
    await setup();
    code = await body();
  } catch (e) {
    log(`[fixtures] ${e.message}`);
    code = 1;
  } finally {
    try {
      await teardown();
    } catch (e) {
      log(`[fixtures] cleanup failed: ${e.message}`);
      code = code || 1;
    }
  }
  return code;
}

/** Exit code of a command (never rejects on a non-zero exit). */
export function exitCode(cmd, args, opts = {}) {
  return new Promise((resolve, reject) => {
    const child = spawnCmd(cmd, args, { cwd: root, stdio: "inherit", ...opts });
    child.on("exit", (code) => resolve(code ?? 1));
    child.on("error", reject);
  });
}

export function loadEnvFile(rel) {
  const p = join(root, rel);
  return existsSync(p) ? parseEnv(readFileSync(p, "utf8")) : {};
}

// npm/pnpm are .cmd shims on Windows and need a shell; native executables do not.
export const needsShell = (cmd) => process.platform === "win32" && ["pnpm", "npm", "npx"].includes(cmd);

export const quote = (a) => (/^[\w.:/\\@=+-]+$/.test(a) ? a : `"${a.replace(/"/g, '\\"')}"`);

/** spawn() that handles Windows .cmd shims without passing an args array to a shell. */
export function spawnCmd(cmd, args, opts) {
  return needsShell(cmd)
    ? spawn([cmd, ...args.map(quote)].join(" "), { ...opts, shell: true })
    : spawn(cmd, args, opts);
}

export function run(cmd, args, opts = {}) {
  return new Promise((resolve, reject) => {
    const child = spawnCmd(cmd, args, { cwd: root, stdio: "inherit", ...opts });
    child.on("exit", (code) => (code === 0 ? resolve() : reject(new Error(`${cmd} ${args.join(" ")} exited with ${code}`))));
    child.on("error", reject);
  });
}

export function compose(args, extra = []) {
  const infra = join(root, ".karbird", "infra.env");
  if (!existsSync(infra)) throw new Error("missing .karbird/infra.env — run `just bootstrap` first");
  return run("docker", ["compose", "--env-file", infra, "-f", join(root, "infra/compose/compose.yaml"), ...extra, ...args]);
}

// PID files let `just stop` clean up children that outlive a force-killed supervisor
// (closing a terminal or `taskkill /F` on it skips the SIGINT handler below).
const runDir = join(root, ".karbird", "run");
const pidFile = (group) => join(runDir, `${group}.json`);
// Only these images are ever killed from a PID file, so a reused PID of another program is safe.
const OURS = /^(node|cmd|sh|bash|ngrok|karbird-[a-z]+)(\.exe)?$/i;

function processName(pid) {
  try {
    if (process.platform === "win32") {
      const out = execFileSync("tasklist", ["/FI", `PID eq ${pid}`, "/FO", "CSV", "/NH"], { encoding: "utf8" });
      const m = out.match(/^"([^"]+)","(\d+)"/m);
      return m && Number(m[2]) === pid ? m[1] : null;
    }
    const out = execFileSync("ps", ["-p", String(pid), "-o", "comm="], { encoding: "utf8" }).trim();
    return out ? basename(out) : null;
  } catch {
    return null;
  }
}

/** Recorded processes of a group that are still alive. */
export function running(group) {
  if (!existsSync(pidFile(group))) return [];
  let entries = [];
  try {
    entries = JSON.parse(readFileSync(pidFile(group), "utf8"));
  } catch {
    return [];
  }
  return entries.filter((e) => Number.isInteger(e.pid) && OURS.test(processName(e.pid) ?? ""));
}

/** Kills the recorded processes of a group (with their children) and removes its PID file. */
export function stopGroup(group) {
  const live = running(group);
  for (const e of live) {
    if (process.platform === "win32") {
      try {
        execFileSync("taskkill", ["/PID", String(e.pid), "/T", "/F"], { stdio: "ignore" });
      } catch {
        // already gone
      }
    } else {
      try {
        execFileSync("pkill", ["-TERM", "-P", String(e.pid)], { stdio: "ignore" });
      } catch {
        // no children
      }
      try {
        process.kill(e.pid, "SIGTERM");
      } catch {
        // already gone
      }
    }
  }
  rmSync(pidFile(group), { force: true });
  return live;
}

/** Exits with a hint when a previous `just dev`/`just demo` is still running. */
export function assertNotRunning(group) {
  const live = running(group);
  if (live.length) {
    console.error(`[${group}] already running (${live.map((e) => `${e.name}:${e.pid}`).join(", ")}). Stop it first: just stop`);
    process.exit(2);
  }
}

function kill(child) {
  if (process.platform === "win32") spawn("taskkill", ["/pid", String(child.pid), "/T", "/F"]);
  else child.kill("SIGTERM");
}

const exited = (child) => child.exitCode !== null || child.signalCode !== null;

/**
 * Run several long-lived processes with prefixed output; stop all on Ctrl+C or when one exits
 * on its own. Returns a handle to stop and start single processes: a process stopped through
 * the handle does not take the others down. The PID file always lists the live processes.
 */
export function supervise(procs, group, { exit = (code) => process.exit(code), signals = true, quiet = false } = {}) {
  const colors = [36, 33, 35, 32, 34, 31];
  const live = new Map();
  let started = 0;
  let stopping = false;

  const writePids = () => {
    if (!group || stopping) return;
    mkdirSync(runDir, { recursive: true });
    writeFileSync(pidFile(group), JSON.stringify([...live.values()].map((e) => ({ name: e.proc.name, pid: e.child.pid }))));
  };

  function start(p) {
    if (stopping) throw new Error("supervisor is shutting down");
    if (live.has(p.name)) throw new Error(`${p.name} is already running`);
    const child = spawnCmd(p.cmd, p.args, {
      cwd: p.cwd ?? root,
      env: { ...process.env, ...(p.env ?? {}) },
    });
    const tag = `\x1b[${colors[started++ % colors.length]}m[${p.name}]\x1b[0m `;
    const pipe = (stream, out) => {
      let buf = "";
      stream.on("data", (d) => {
        buf += d.toString();
        const lines = buf.split(/\r?\n/);
        buf = lines.pop() ?? "";
        if (!quiet) for (const l of lines) out.write(tag + l + "\n");
      });
    };
    pipe(child.stdout, process.stdout);
    pipe(child.stderr, process.stderr);
    const entry = { proc: p, child, expected: false };
    child.on("exit", (code) => {
      if (!quiet) process.stderr.write(`${tag}exited with ${code}${entry.expected ? " (stopped)" : ""}\n`);
      if (live.get(p.name) === entry) live.delete(p.name);
      writePids();
      if (!entry.expected && !p.optional) shutdown(code ?? 1);
    });
    live.set(p.name, entry);
    writePids();
    return child;
  }

  /** Stops one process on purpose; resolves once it has exited. */
  function stop(name) {
    const entry = live.get(name);
    if (!entry) return Promise.resolve(false);
    entry.expected = true;
    return new Promise((resolve) => {
      if (exited(entry.child)) return resolve(true);
      entry.child.once("exit", () => resolve(true));
      kill(entry.child);
    });
  }

  function shutdown(code) {
    if (stopping) return;
    stopping = true;
    if (group) rmSync(pidFile(group), { force: true });
    for (const { child } of live.values()) {
      if (!exited(child)) kill(child);
    }
    setTimeout(() => exit(code), 1500);
  }

  for (const p of procs) start(p);
  if (signals) {
    process.on("SIGINT", () => shutdown(0));
    process.on("SIGTERM", () => shutdown(0));
  }
  return {
    start,
    stop,
    shutdown,
    running: (name) => live.has(name),
    pid: (name) => live.get(name)?.child.pid ?? null,
  };
}
