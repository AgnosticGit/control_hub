import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { fileURLToPath } from 'node:url';

export const src = (name) => fileURLToPath(new URL(`../src/${name}`, import.meta.url));

/** Env of this process without DEMO_* / CONTROLHUB_* so tests never reach the running demo. */
export function cleanEnv(extra = {}) {
  const env = {};
  for (const [k, v] of Object.entries(process.env)) {
    if (!k.startsWith('DEMO_') && !k.startsWith('CONTROLHUB_')) env[k] = v;
  }
  return { ...env, ...extra };
}

/** Local HTTP server recording requests; `handler` may override the reply. */
export async function captureServer(handler) {
  const requests = [];
  const server = createServer((req, res) => {
    const chunks = [];
    req.on('data', (c) => chunks.push(c));
    req.on('end', () => {
      const entry = { method: req.method, url: req.url, headers: req.headers, body: Buffer.concat(chunks).toString('utf8') };
      requests.push(entry);
      if (handler) return handler(entry, res);
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end('{"ok":true}');
    });
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  return {
    requests,
    url: `http://127.0.0.1:${server.address().port}`,
    close: () => new Promise((r) => server.close(() => r())),
  };
}

export async function freePort() {
  const s = createServer();
  await new Promise((r) => s.listen(0, '127.0.0.1', r));
  const { port } = s.address();
  await new Promise((r) => s.close(() => r()));
  return port;
}

/** Runs a script to completion. */
export function runNode(script, args, env, timeoutMs = 15_000) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [script, ...args], { env });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (d) => (stdout += d));
    child.stderr.on('data', (d) => (stderr += d));
    const timer = setTimeout(() => {
      child.kill();
      reject(new Error(`timed out: ${script} ${args.join(' ')}`));
    }, timeoutMs);
    child.on('error', reject);
    child.on('exit', (code) => {
      clearTimeout(timer);
      resolve({ code, stdout, stderr });
    });
  });
}

/** Starts a long-running script and resolves once `pattern` appears on stdout. */
export function startNode(script, env, pattern, timeoutMs = 15_000) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [script], { env });
    let stdout = '';
    const stop = () =>
      new Promise((r) => {
        if (child.exitCode !== null) return r();
        child.once('exit', () => r());
        child.kill();
      });
    const timer = setTimeout(() => {
      child.kill();
      reject(new Error(`no ${pattern} from ${script}; got: ${stdout}`));
    }, timeoutMs);
    child.stdout.on('data', (d) => {
      stdout += d;
      if (pattern.test(stdout)) {
        clearTimeout(timer);
        resolve({ child, stop, output: () => stdout });
      }
    });
    child.on('error', reject);
  });
}
