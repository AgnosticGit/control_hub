// Demo-only fault control:
//   node src/fault.mjs orders --latency 800 --error-rate 0.3
//   node src/fault.mjs orders --clear
//   node src/fault.mjs gateway --status

const targets = {
  gateway: process.env.DEMO_GATEWAY_URL ?? 'http://127.0.0.1:7001',
  orders: process.env.DEMO_ORDERS_URL ?? 'http://127.0.0.1:7002',
};

const [service, ...rest] = process.argv.slice(2);
if (!service || !(service in targets)) {
  console.error('usage: fault.mjs <gateway|orders> [--latency MS] [--error-rate 0..1] [--clear] [--status]');
  process.exit(2);
}
const opts = {};
for (let i = 0; i < rest.length; i++) {
  const a = rest[i];
  if (a === '--latency') opts.latencyMs = Number(rest[++i]);
  else if (a === '--error-rate') opts.errorRate = Number(rest[++i]);
  else if (a === '--clear') Object.assign(opts, { latencyMs: 0, errorRate: 0 });
  else if (a === '--status') opts.status = true;
  else {
    console.error(`unknown argument: ${a}`);
    process.exit(2);
  }
}

const url = `${targets[service]}/__demo/faults`;
const res = opts.status
  ? await fetch(url)
  : await fetch(url, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ latencyMs: opts.latencyMs ?? 0, errorRate: opts.errorRate ?? 0 }),
    });
const body = await res.text();
if (!res.ok) {
  console.error(`fault control failed (${res.status}): ${body}`);
  console.error('Fault injection requires the demo started with CONTROLHUB_DEMO_FAULTS=1 (just demo does this).');
  process.exit(1);
}
console.log(`${service}: ${body}`);
