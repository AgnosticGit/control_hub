'use strict';
// Shared helpers for demo services. DEMO ONLY — not production code.

const http = require('node:http');

const LOOPBACK = new Set(['127.0.0.1', '::1', '::ffff:127.0.0.1']);

function logger() {
  // The SDK is started by `--import @controlhub/sdk-node/register`; without it
  // the demo still runs and logs to stdout only.
  const handle = globalThis[Symbol.for('controlhub.sdk-node')];
  const fallback = (level) => (msg, attrs) => console.log(JSON.stringify({ level, msg, ...attrs }));
  return {
    info: handle ? handle.logger.info : fallback('info'),
    warn: handle ? handle.logger.warn : fallback('warn'),
    error: handle ? handle.logger.error : fallback('error'),
  };
}

function sendJson(res, status, body) {
  const text = JSON.stringify(body);
  res.writeHead(status, { 'content-type': 'application/json', 'content-length': Buffer.byteLength(text) });
  res.end(text);
}

function readJson(req, limit = 64 * 1024) {
  return new Promise((resolve, reject) => {
    let size = 0;
    const chunks = [];
    req.on('data', (c) => {
      size += c.length;
      if (size > limit) {
        reject(new Error('body too large'));
        req.destroy();
        return;
      }
      chunks.push(c);
    });
    req.on('end', () => {
      if (chunks.length === 0) return resolve({});
      try {
        resolve(JSON.parse(Buffer.concat(chunks).toString('utf8')));
      } catch {
        reject(new Error('invalid JSON'));
      }
    });
    req.on('error', reject);
  });
}

/**
 * Demo-only fault injection state. Enabled only when CONTROLHUB_DEMO_FAULTS=1
 * and only reachable from loopback.
 */
function createFaults() {
  const enabled = process.env.CONTROLHUB_DEMO_FAULTS === '1';
  const state = { latencyMs: 0, errorRate: 0 };
  return {
    enabled,
    state,
    async apply() {
      if (!enabled) return false;
      if (state.latencyMs > 0) await new Promise((r) => setTimeout(r, state.latencyMs));
      return Math.random() < state.errorRate;
    },
    /** Returns true when the request was handled. */
    async handle(req, res) {
      if (!req.url || !req.url.startsWith('/__demo/faults')) return false;
      if (!enabled || !LOOPBACK.has(req.socket.remoteAddress ?? '')) {
        sendJson(res, 404, { error: 'not found' });
        return true;
      }
      if (req.method === 'GET') {
        sendJson(res, 200, state);
        return true;
      }
      if (req.method === 'POST') {
        let body;
        try {
          body = await readJson(req);
        } catch (e) {
          sendJson(res, 400, { error: String(e.message) });
          return true;
        }
        const latency = Number(body.latencyMs ?? 0);
        const rate = Number(body.errorRate ?? 0);
        if (!Number.isFinite(latency) || latency < 0 || latency > 30_000 || !Number.isFinite(rate) || rate < 0 || rate > 1) {
          sendJson(res, 400, { error: 'latencyMs must be 0..30000 and errorRate 0..1' });
          return true;
        }
        state.latencyMs = latency;
        state.errorRate = rate;
        sendJson(res, 200, state);
        return true;
      }
      sendJson(res, 405, { error: 'method not allowed' });
      return true;
    },
  };
}

function getJson(url, timeoutMs = 5000) {
  return new Promise((resolve, reject) => {
    const req = http.get(url, { timeout: timeoutMs }, (res) => {
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => {
        const text = Buffer.concat(chunks).toString('utf8');
        let body = null;
        try {
          body = JSON.parse(text);
        } catch {
          body = { raw: text.slice(0, 200) };
        }
        resolve({ status: res.statusCode ?? 0, body });
      });
    });
    req.on('timeout', () => req.destroy(new Error('upstream timeout')));
    req.on('error', reject);
  });
}

/** HTTP call with an optional JSON body; resolves with the status and parsed body. */
function callJson(method, url, body, timeoutMs = 5000) {
  return new Promise((resolve, reject) => {
    const text = body === undefined ? null : JSON.stringify(body);
    const headers = text === null ? {} : { 'content-type': 'application/json', 'content-length': Buffer.byteLength(text) };
    const req = http.request(url, { method, timeout: timeoutMs, headers }, (res) => {
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => {
        let parsed = null;
        try {
          parsed = chunks.length ? JSON.parse(Buffer.concat(chunks).toString('utf8')) : null;
        } catch {
          parsed = null;
        }
        resolve({ status: res.statusCode ?? 0, body: parsed });
      });
    });
    req.on('timeout', () => req.destroy(new Error('upstream timeout')));
    req.on('error', reject);
    req.end(text ?? undefined);
  });
}

/** Busy-waits for `ms` to stand in for real CPU work in a demo handler. */
function work(ms) {
  const until = performance.now() + ms;
  while (performance.now() < until) {
    // intentionally empty
  }
}

function listen(server, port, name) {
  server.listen(port, '127.0.0.1', () => console.log(`${name} listening on http://127.0.0.1:${port}`));
  server.requestTimeout = 30_000;
}

/**
 * A small demo service: health, fault injection and injected failures are handled here;
 * `route` handles the rest and returns false for "not found".
 */
function serve(name, port, route) {
  const log = logger();
  const faults = createFaults();
  const server = http.createServer(async (req, res) => {
    try {
      if (await faults.handle(req, res)) return;
      const url = new URL(req.url ?? '/', 'http://localhost');
      if (url.pathname === '/health') return sendJson(res, 200, { ok: true });
      if (await faults.apply()) {
        log.error(`injected ${name} failure`, { 'demo.fault': 'error' });
        return sendJson(res, 500, { error: 'injected demo failure' });
      }
      if ((await route(req, res, url, log)) === false) sendJson(res, 404, { error: 'not found' });
    } catch (err) {
      log.error(`${name} request failed`, { 'error.type': err?.name ?? 'Error' });
      if (!res.headersSent) sendJson(res, 500, { error: 'internal error' });
    }
  });
  listen(server, port, name);
  return server;
}

module.exports = { logger, sendJson, readJson, createFaults, getJson, callJson, work, listen, serve };
