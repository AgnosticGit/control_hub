'use strict';
// Demo "inventory" service: stock levels in the demo Postgres. DEMO ONLY.

const { Pool } = require('pg');
const { logger, sendJson, serve } = require('./common.cjs');

const log = logger();
const pool = new Pool({
  host: '127.0.0.1',
  port: Number(process.env.CH_DEMO_PG_PORT ?? 15433),
  user: 'demo',
  database: 'orders',
  password: process.env.CH_DEMO_PG_PASSWORD,
  max: 5,
  connectionTimeoutMillis: 5000,
  statement_timeout: 5000,
});
pool.on('error', (err) => log.warn('idle database connection lost', { 'error.type': err?.name ?? 'Error' }));

async function migrate() {
  await pool.query(`CREATE TABLE IF NOT EXISTS stock (
    item_id INT PRIMARY KEY,
    quantity INT NOT NULL CHECK (quantity >= 0)
  )`);
  await pool.query(
    `INSERT INTO stock (item_id, quantity) SELECT g, 500 FROM generate_series(1, 60) g ON CONFLICT (item_id) DO NOTHING`,
  );
}

function start() {
  serve('inventory', Number(process.env.DEMO_INVENTORY_PORT ?? 7003), async (req, res, url, l) => {
    const match = url.pathname.match(/^\/stock\/(\d{1,9})$/);
    if (req.method === 'GET' && match) {
      const { rows } = await pool.query('SELECT quantity FROM stock WHERE item_id = $1', [Number(match[1])]);
      return sendJson(res, 200, { quantity: rows[0]?.quantity ?? 0 });
    }
    if (req.method === 'POST' && url.pathname === '/reserve') {
      const item = 1 + Math.floor(Math.random() * 60);
      // Restock instead of running out, so the demo keeps flowing.
      const { rows } = await pool.query(
        'UPDATE stock SET quantity = CASE WHEN quantity > 0 THEN quantity - 1 ELSE 500 END WHERE item_id = $1 RETURNING quantity',
        [item],
      );
      l.info('stock reserved', { 'item.id': item });
      return sendJson(res, 200, { item, quantity: rows[0]?.quantity ?? 0 });
    }
    return false;
  });
}

migrate()
  .then(start)
  .catch((err) => {
    console.error('inventory: database migration failed:', err.message);
    process.exit(1);
  });
