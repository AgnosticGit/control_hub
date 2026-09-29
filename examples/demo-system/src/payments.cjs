'use strict';
// Demo "payments" service: charges through an external payment provider (the provider is
// not instrumented, so ControlHub infers it from the client spans). DEMO ONLY.

const { sendJson, callJson, work, serve } = require('./common.cjs');

const providerUrl = process.env.DEMO_PROVIDER_URL ?? 'http://127.0.0.1:7010';

serve('payments', Number(process.env.DEMO_PAYMENTS_PORT ?? 7004), async (req, res, url, log) => {
  if (req.method === 'POST' && url.pathname === '/charge') {
    work(0.2 + Math.random() * 0.3);
    const charge = await callJson('POST', `${providerUrl}/v1/charges`, { amount: 1 + Math.floor(Math.random() * 100) });
    if (charge.status >= 500) {
      log.error('payment provider failed', { 'http.response.status_code': charge.status });
      return sendJson(res, 502, { error: 'provider failure' });
    }
    if (charge.status === 402) {
      log.warn('payment declined');
      return sendJson(res, 402, { error: 'declined' });
    }
    log.info('payment captured');
    return sendJson(res, 200, { captured: true });
  }
  return false;
});
