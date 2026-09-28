import { spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
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

export function loadEnvFile(rel) {
  const p = join(root, rel);
  return existsSync(p) ? parseEnv(readFileSync(p, "utf8")) : {};
}

// npm/pnpm are .cmd shims on Windows and need a shell; native executables do not.
export const needsShell = (cmd) => process.platform === "win32" && ["pnpm", "npm", "npx"].includes(cmd);

const quote = (a) => (/^[\w.:/\\@=+-]+$/.test(a) ? a : `"${a.replace(/"/g, '\\"')}"`);

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
  const infra = join(root, ".controlhub", "infra.env");
  if (!existsSync(infra)) throw new Error("missing .controlhub/infra.env — run `just bootstrap` first");
  return run("docker", ["compose", "--env-file", infra, "-f", join(root, "infra/compose/compose.yaml"), ...extra, ...args]);
}

/** Run several long-lived processes with prefixed output; stop all on Ctrl+C or first failure. */
export function supervise(procs) {
  const colors = [36, 33, 35, 32, 34, 31];
  const children = procs.map((p, i) => {
    const child = spawnCmd(p.cmd, p.args, {
      cwd: p.cwd ?? root,
      env: { ...process.env, ...(p.env ?? {}) },
    });
    const tag = `\x1b[${colors[i % colors.length]}m[${p.name}]\x1b[0m `;
    const pipe = (stream, out) => {
      let buf = "";
      stream.on("data", (d) => {
        buf += d.toString();
        const lines = buf.split(/\r?\n/);
        buf = lines.pop() ?? "";
        for (const l of lines) out.write(tag + l + "\n");
      });
    };
    pipe(child.stdout, process.stdout);
    pipe(child.stderr, process.stderr);
    child.on("exit", (code) => {
      process.stderr.write(`${tag}exited with ${code}\n`);
      if (!p.optional) shutdown(code ?? 1);
    });
    return child;
  });
  let stopping = false;
  function shutdown(code) {
    if (stopping) return;
    stopping = true;
    for (const c of children) {
      if (c.exitCode === null) {
        if (process.platform === "win32") spawn("taskkill", ["/pid", String(c.pid), "/T", "/F"]);
        else c.kill("SIGTERM");
      }
    }
    setTimeout(() => process.exit(code), 1500);
  }
  process.on("SIGINT", () => shutdown(0));
  process.on("SIGTERM", () => shutdown(0));
}
