'use strict';
// Demo "orders" service: HTTP API backed by the demo Postgres. DEMO ONLY.

const http = require('node:http');
const { Pool } = require('pg');
const { logger, sendJson, createFaults, listen } = require('./common.cjs');

const port = Number(process.env.DEMO_ORDERS_PORT ?? 7002);
const log = logger();
const faults = createFaults();

const pool = new Pool({
  host: '127.0.0.1',
  port: Number(process.env.CH_DEMO_PG_PORT ?? 15433),
  user: 'demo',
  database: 'orders',
  password: process.env.CH_DEMO_PG_PASSWORD,
  max: 10,
  connectionTimeoutMillis: 5000,
  statement_timeout: 5000,
});

async function migrate() {
  await pool.query(`CREATE TABLE IF NOT EXISTS orders (
    id BIGSERIAL PRIMARY KEY,
    item TEXT NOT NULL,
    quantity INT NOT NULL CHECK (quantity > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
  )`);
  const { rows } = await pool.query('SELECT count(*)::int AS n FROM orders');
  if (rows[0].n === 0) {
    await pool.query(
      `INSERT INTO orders (item, quantity) SELECT 'item-' || g, 1 + (g % 5) FROM generate_series(1, 50) g`,
    );
  }
}

const server = http.createServer(async (req, res) => {
  try {
    if (await faults.handle(req, res)) return;
    const url = new URL(req.url ?? '/', 'http://localhost');
    if (url.pathname === '/health') return sendJson(res, 200, { ok: true });

    if (await faults.apply()) {
      log.error('injected failure while loading order', { 'demo.fault': 'error' });
      return sendJson(res, 500, { error: 'injected demo failure' });
    }

    const match = url.pathname.match(/^\/orders\/(\d{1,9})$/);
    if (req.method === 'GET' && match) {
      const id = Number(match[1]);
      const { rows } = await pool.query('SELECT id, item, quantity FROM orders WHERE id = $1', [id]);
      if (rows.length === 0) {
        log.warn('order not found', { 'order.id': id });
        return sendJson(res, 404, { error: 'order not found' });
      }
      log.info('order loaded', { 'order.id': id });
      return sendJson(res, 200, rows[0]);
    }
    if (req.method === 'POST' && url.pathname === '/orders') {
      const quantity = 1 + Math.floor(Math.random() * 5);
      const { rows } = await pool.query(
        'INSERT INTO orders (item, quantity) VALUES ($1, $2) RETURNING id',
        [`item-${Date.now() % 1000}`, quantity],
      );
      log.info('order created', { 'order.id': Number(rows[0].id) });
      return sendJson(res, 201, { id: Number(rows[0].id) });
    }
    return sendJson(res, 404, { error: 'not found' });
  } catch (err) {
    log.error('request failed', { 'error.type': err?.name ?? 'Error' });
    if (!res.headersSent) sendJson(res, 500, { error: 'internal error' });
  }
});

migrate()
  .then(() => listen(server, port, 'orders'))
  .catch((err) => {
    console.error('orders: database migration failed:', err.message);
    process.exit(1);
  });
