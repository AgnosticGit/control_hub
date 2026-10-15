// Demo load generator: steady mix of reads and writes against the gateway.
// Not instrumented on purpose — it plays the role of external clients.
import http from 'node:http';

const target = process.env.DEMO_GATEWAY_URL ?? 'http://127.0.0.1:7001';
// A non-numeric DEMO_RPS would give a NaN interval, which setInterval runs as fast as it can.
const defaultRps = 10 / 60;
const requestedRps = Number(process.env.DEMO_RPS ?? defaultRps);
const rps = Math.max(1 / 60, Math.min(200, Number.isFinite(requestedRps) ? requestedRps : defaultRps));
const counts = { ok: 0, clientError: 0, serverError: 0, failed: 0 };
let incidentActive = false;
const incidentTarget = process.env.DEMO_ORDERS_URL ?? 'http://127.0.0.1:7002';

const control = http.createServer((req, res) => {
  if (req.method !== 'POST' || req.url !== '/__demo/incident-state') {
    res.writeHead(404).end();
    return;
  }
  let body = '';
  req.on('data', (chunk) => { body += chunk; if (body.length > 128) req.destroy(); });
  req.on('end', () => {
    try {
      const value = JSON.parse(body);
      if (typeof value.active !== 'boolean') throw new Error('invalid state');
      incidentActive = value.active;
      res.writeHead(204).end();
    } catch {
      res.writeHead(400).end();
    }
  });
});
control.listen(Number(process.env.DEMO_LOAD_CONTROL_PORT ?? 7091), '127.0.0.1', () => {
  console.log(`load control: ${control.address().port}`);
});

async function once() {
  // 15 % order writes, 30 % catalog reads, the rest order reads.
  const r = Math.random();
  const write = r < 0.15;
  const probe = incidentActive;
  const url = probe
    ? `${incidentTarget}/__demo/incident-probe`
    : write
    ? `${target}/api/orders`
    : r < 0.45
      ? `${target}/api/catalog/${1 + Math.floor(Math.random() * 60)}`
      : `${target}/api/orders/${1 + Math.floor(Math.random() * 55)}`;
  try {
    const res = await fetch(url, { method: !probe && write ? 'POST' : 'GET', signal: AbortSignal.timeout(10_000) });
    await res.body?.cancel();
    if (res.status >= 500) counts.serverError++;
    else if (res.status >= 400) counts.clientError++;
    else counts.ok++;
  } catch {
    counts.failed++;
  }
}

setInterval(() => void once(), 1000 / rps);
setInterval(() => console.log(`load: ${JSON.stringify(counts)}`), 10_000).unref();
console.log(`load generator: ${rps} req/s -> ${target}`);
