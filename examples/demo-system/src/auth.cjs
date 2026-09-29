'use strict';
// Demo "auth" service: the gateway asks it to verify every request. DEMO ONLY: nothing is
// actually verified; it only spends a little CPU like a signature check would.

const { sendJson, work, serve } = require('./common.cjs');

serve('auth', Number(process.env.DEMO_AUTH_PORT ?? 7007), async (req, res, url) => {
  if (req.method === 'GET' && url.pathname === '/verify') {
    work(0.2 + Math.random() * 0.4);
    return sendJson(res, 200, { ok: true });
  }
  return false;
});
