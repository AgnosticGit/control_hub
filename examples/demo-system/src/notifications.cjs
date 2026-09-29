'use strict';
// Demo "notifications" service: accepts an order notification and answers at once; the
// "delivery" is only a log line. DEMO ONLY.

const { sendJson, readJson, work, serve } = require('./common.cjs');

serve('notifications', Number(process.env.DEMO_NOTIFICATIONS_PORT ?? 7005), async (req, res, url, log) => {
  if (req.method === 'POST' && url.pathname === '/notify') {
    const body = await readJson(req);
    work(0.1 + Math.random() * 0.2);
    log.info('notification queued', { 'order.id': Number(body.orderId ?? 0) });
    return sendJson(res, 202, { queued: true });
  }
  return false;
});
