// Test fixture, run as `node --import ../../src/register.js instrumented-app.mjs --secret-arg=…`.
// Serves one request carrying credentials and a body, then prints diagnostics and shuts down.
// Requests are sent with `fetch` (undici, not instrumented), so every exported span is a server span.
import { createServer } from 'node:http';
import { getControlHub } from '../../src/index.js';

const handle = getControlHub();
if (!handle) throw new Error('SDK not started by register');

const server = createServer((req, res) => {
  const chunks = [];
  req.on('data', (c) => chunks.push(c));
  req.on('end', () => {
    res.writeHead(200, { 'content-type': 'application/json', 'set-cookie': 'session=resp-cookie-secret; HttpOnly' });
    res.end(JSON.stringify({ echoedBytes: Buffer.concat(chunks).length }));
  });
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;

const res = await fetch(`${base}/orders?q=1`, {
  method: 'POST',
  headers: {
    authorization: 'Bearer user-secret-AUTH',
    cookie: 'session=cookie-secret-COOKIE',
    'x-api-key': 'apikey-secret-KEY',
    'content-type': 'application/json',
  },
  body: JSON.stringify({ password: 'body-secret-PW', card: '4111111111111111' }),
});
await res.text();
await (await fetch(`${base}/health`)).text();

handle.logger.info('fixture log line', { 'fixture.attr': 'visible' });
handle.events.emit('fixture.custom', { attributes: { stage: 'done' } });

console.log(JSON.stringify(handle.diagnostics()));
server.close();
await handle.shutdown();
