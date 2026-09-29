'use strict';
// Demo "gateway" service: public-facing HTTP API that calls the orders
// service. DEMO ONLY.

const http = require('node:http');
const { logger, sendJson, createFaults, getJson, callJson, listen } = require('./common.cjs');

const port = Number(process.env.DEMO_GATEWAY_PORT ?? 7001);
const ordersUrl = process.env.DEMO_ORDERS_URL ?? 'http://127.0.0.1:7002';
const authUrl = process.env.DEMO_AUTH_URL ?? 'http://127.0.0.1:7007';
const catalogUrl = process.env.DEMO_CATALOG_URL ?? 'http://127.0.0.1:7006';
const log = logger();
const faults = createFaults();

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

const server = http.createServer(async (req, res) => {
  try {
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
      const upstream = await getJson(`${catalogUrl}/items/${item[1]}`);
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
