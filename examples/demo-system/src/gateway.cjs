'use strict';
// Demo "gateway" service: public-facing HTTP API that calls the orders
// service. DEMO ONLY.

const http = require('node:http');
const { logger, sendJson, readJson, createFaults, getJson, callJson, listen, isLoopback } = require('./common.cjs');

const port = Number(process.env.DEMO_GATEWAY_PORT ?? 7001);
const ordersUrl = process.env.DEMO_ORDERS_URL ?? 'http://127.0.0.1:7002';
const authUrl = process.env.DEMO_AUTH_URL ?? 'http://127.0.0.1:7007';
const CATALOG_CACHE_MS = 10_000;
const MAX_UPSTREAMS = 10;
const UPSTREAM = /^http:\/\/127\.0\.0\.1:\d{2,5}$/;

// Catalog runs as several replicas: requests go round-robin over DEMO_CATALOG_URLS. The demo
// orchestrator changes the list when it scales or restarts replicas (`/__demo/upstreams`).
let catalogUrls = (process.env.DEMO_CATALOG_URLS ?? process.env.DEMO_CATALOG_URL ?? 'http://127.0.0.1:7006')
  .split(',')
  .map((u) => u.trim())
  .filter(Boolean);
let nextCatalog = 0;
const catalogUrl = () => catalogUrls[nextCatalog++ % catalogUrls.length];
const log = logger();
const faults = createFaults({ flags: ['catalog-cache'] });
/** `catalog-cache` flag: item id → upstream reply, kept for CATALOG_CACHE_MS. */
const catalogCache = new Map();

function postJson(url) {
  return new Promise((resolve, reject) => {
    const req = http.request(url, { method: 'POST', timeout: 5000 }, (res) => {
      res.resume();
      res.on('end', () => resolve({ status: res.statusCode ?? 0 }));
    });
    req.on('timeout', () => req.destroy(new Error('upstream timeout')));
    req.on('error', reject);
    req.end();
  });
}

/** Demo-only: read or replace the catalog replica list (loopback, fault control enabled). */
async function handleUpstreams(req, res) {
  if (!faults.enabled || !isLoopback(req)) return sendJson(res, 404, { error: 'not found' });
  if (req.method === 'GET') return sendJson(res, 200, { catalog: catalogUrls });
  if (req.method !== 'POST') return sendJson(res, 405, { error: 'method not allowed' });
  let body;
  try {
    body = await readJson(req);
  } catch (e) {
    return sendJson(res, 400, { error: String(e.message) });
  }
  const list = body.catalog;
  if (!Array.isArray(list) || list.length === 0 || list.length > MAX_UPSTREAMS || !list.every((u) => typeof u === 'string' && UPSTREAM.test(u))) {
    return sendJson(res, 400, { error: `catalog must be 1..${MAX_UPSTREAMS} URLs of the form http://127.0.0.1:<port>` });
  }
  catalogUrls = [...new Set(list)];
  nextCatalog = 0;
  log.info('catalog upstreams changed', { 'demo.upstreams': catalogUrls.length });
  return sendJson(res, 200, { catalog: catalogUrls });
}

async function catalogItem(id) {
  if (faults.flag('catalog-cache')) {
    const hit = catalogCache.get(id);
    if (hit && hit.expires > Date.now()) return hit.reply;
  } else if (catalogCache.size > 0) {
    catalogCache.clear();
  }
  const reply = await getJson(`${catalogUrl()}/items/${id}`);
  if (faults.flag('catalog-cache') && reply.status === 200) catalogCache.set(id, { reply, expires: Date.now() + CATALOG_CACHE_MS });
  return reply;
}

const server = http.createServer(async (req, res) => {
  try {
    if (req.url === '/__demo/upstreams') return await handleUpstreams(req, res);
    if (await faults.handle(req, res)) return;
    const url = new URL(req.url ?? '/', 'http://localhost');
    if (url.pathname === '/health') return sendJson(res, 200, { ok: true });

    if (await faults.apply()) {
      log.error('injected gateway failure', { 'demo.fault': 'error' });
      return sendJson(res, 502, { error: 'injected demo failure' });
    }

    if (url.pathname.startsWith('/api/')) {
      const auth = await callJson('GET', `${authUrl}/verify`);
      if (auth.status !== 200) {
        log.error('auth check failed', { 'http.response.status_code': auth.status });
        return sendJson(res, 502, { error: 'auth unavailable' });
      }
    }

    const item = url.pathname.match(/^\/api\/catalog\/(\d{1,9})$/);
    if (req.method === 'GET' && item) {
      const upstream = await catalogItem(item[1]);
      if (upstream.status >= 500) {
        log.error('catalog service failed', { 'http.response.status_code': upstream.status });
        return sendJson(res, 502, { error: 'upstream failure' });
      }
      return sendJson(res, upstream.status, upstream.body);
    }

    const match = url.pathname.match(/^\/api\/orders\/(\d{1,9})$/);
    if (req.method === 'GET' && match) {
      const upstream = await getJson(`${ordersUrl}/orders/${match[1]}`);
      if (upstream.status >= 500) {
        log.error('orders service failed', { 'http.response.status_code': upstream.status });
        return sendJson(res, 502, { error: 'upstream failure' });
      }
      return sendJson(res, upstream.status, upstream.body);
    }
    if (req.method === 'POST' && url.pathname === '/api/orders') {
      const upstream = await postJson(`${ordersUrl}/orders`);
      if (upstream.status >= 500) {
        log.error('order creation failed', { 'http.response.status_code': upstream.status });
        return sendJson(res, 502, { error: 'upstream failure' });
      }
      if (upstream.status === 402) return sendJson(res, 402, { error: 'payment declined' });
      log.info('order accepted');
      return sendJson(res, 202, { accepted: true });
    }
    return sendJson(res, 404, { error: 'not found' });
  } catch (err) {
    log.error('gateway request failed', { 'error.type': err?.name ?? 'Error' });
    if (!res.headersSent) sendJson(res, 502, { error: 'upstream unavailable' });
  }
});

listen(server, port, 'gateway');
