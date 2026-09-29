'use strict';
// Demo "catalog" service: item details, with the stock level from inventory. DEMO ONLY.

const { sendJson, callJson, work, serve } = require('./common.cjs');

const inventoryUrl = process.env.DEMO_INVENTORY_URL ?? 'http://127.0.0.1:7003';

serve('catalog', Number(process.env.DEMO_CATALOG_PORT ?? 7006), async (req, res, url, log) => {
  const match = url.pathname.match(/^\/items\/(\d{1,9})$/);
  if (req.method === 'GET' && match) {
    const id = Number(match[1]);
    work(0.3 + Math.random() * 0.5);
    const stock = await callJson('GET', `${inventoryUrl}/stock/${id}`);
    if (stock.status >= 500) {
      log.error('inventory unavailable', { 'http.response.status_code': stock.status });
      return sendJson(res, 502, { error: 'upstream failure' });
    }
    return sendJson(res, 200, { id, name: `item-${id}`, available: stock.body?.quantity ?? 0 });
  }
  return false;
});
