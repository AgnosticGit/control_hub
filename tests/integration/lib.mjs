// Shared helpers for integration tests against a locally running stack.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

export const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
export const creds = JSON.parse(readFileSync(join(root, '.controlhub', 'local-credentials.json'), 'utf8'));
export const API = process.env.CONTROLHUB_API_URL ?? creds.api_url;
export const OTLP = creds.otlp_http_endpoint;
export const ORIGIN = 'http://localhost:3000';
export const WEB = process.env.CONTROLHUB_WEB_URL ?? ORIGIN;
export const user = (email) => creds.users.find((u) => u.email === email);

export class Session {
  constructor() {
    this.cookies = new Map();
  }
  cookieHeader() {
    return [...this.cookies].map(([k, v]) => `${k}=${v}`).join('; ');
  }
  async request(path, init = {}) {
    const headers = { ...(init.headers ?? {}) };
    if (this.cookies.size) headers.cookie = this.cookieHeader();
    const res = await fetch(`${API}${path}`, { ...init, headers });
    for (const c of res.headers.getSetCookie()) {
      const [pair] = c.split(';');
      const i = pair.indexOf('=');
      this.cookies.set(pair.slice(0, i), pair.slice(i + 1));
    }
    return res;
  }
  async json(path, init) {
    const res = await this.request(path, init);
    const body = await res.json().catch(() => null);
    return { status: res.status, body };
  }
  /** Cookie-authenticated mutation with Origin and CSRF header. */
  async mutate(path, body, method = 'POST') {
    return this.json(path, {
      method,
      headers: {
        'content-type': 'application/json',
        origin: ORIGIN,
        'x-csrf-token': this.cookies.get('controlhub_csrf') ?? '',
      },
      body: JSON.stringify(body ?? {}),
    });
  }
  async login(email) {
    const u = user(email);
    const r = await this.json('/api/v1/session/login', {
      method: 'POST',
      headers: { 'content-type': 'application/json', origin: ORIGIN },
      body: JSON.stringify({ email: u.email, password: u.password }),
    });
    assert.equal(r.status, 200, `login ${email}: ${JSON.stringify(r.body)}`);
    return r.body;
  }
}

export const scoped = (project, env) => `/api/v1/projects/${project}/environments/${env}`;
export const DEMO = scoped('demo', 'local-demo');
export const ISO = scoped('isolation', 'isolation-test');

export async function eventually(fn, { timeoutMs = 30_000, intervalMs = 1000 } = {}) {
  const deadline = Date.now() + timeoutMs;
  let last;
  while (Date.now() < deadline) {
    try {
      return await fn();
    } catch (e) {
      last = e;
      await new Promise((r) => setTimeout(r, intervalMs));
    }
  }
  throw last;
}

export async function bearer(token, path, init = {}) {
  const res = await fetch(`${API}${path}`, {
    ...init,
    headers: { ...(init.headers ?? {}), authorization: `Bearer ${token}` },
  });
  return { status: res.status, body: await res.json().catch(() => null) };
}
