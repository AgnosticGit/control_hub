// Demo stand-in for an external payment provider. Not instrumented on purpose: karbird
// only sees the payments service's client spans and shows the provider as inferred.
import { createServer } from 'node:http';

const port = Number(process.env.DEMO_PROVIDER_PORT ?? 7010);

const server = createServer((req, res) => {
  req.resume();
  req.on('end', () => {
    // External latency of 3-12 ms; about 2 % of charges are declined.
    const delay = 3 + Math.random() * 9;
    setTimeout(() => {
      const declined = Math.random() < 0.02;
      const body = JSON.stringify(declined ? { status: 'declined' } : { status: 'captured' });
      res.writeHead(declined ? 402 : 200, { 'content-type': 'application/json', 'content-length': Buffer.byteLength(body) });
      res.end(body);
    }, delay);
  });
});
server.listen(port, '127.0.0.1', () => console.log(`payment provider stand-in listening on http://127.0.0.1:${port}`));
