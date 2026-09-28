// Demo load generator: steady mix of reads and writes against the gateway.
// Not instrumented on purpose — it plays the role of external clients.

const target = process.env.DEMO_GATEWAY_URL ?? 'http://127.0.0.1:7001';
const rps = Math.max(0.2, Math.min(200, Number(process.env.DEMO_RPS ?? 5)));
const counts = { ok: 0, clientError: 0, serverError: 0, failed: 0 };

async function once() {
  const write = Math.random() < 0.15;
  const url = write ? `${target}/api/orders` : `${target}/api/orders/${1 + Math.floor(Math.random() * 55)}`;
  try {
    const res = await fetch(url, { method: write ? 'POST' : 'GET', signal: AbortSignal.timeout(10_000) });
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
