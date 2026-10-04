// Demo-only fault control:
//   node src/fault.mjs orders --latency 800 --error-rate 0.3
//   node src/fault.mjs orders --clear
//   node src/fault.mjs gateway --status
//   node src/fault.mjs inventory --cpu-hog 0.95   (noisy neighbour; --cpu-hog 0 stops it)

const targets = {
  gateway: process.env.DEMO_GATEWAY_URL ?? 'http://127.0.0.1:7001',
  orders: process.env.DEMO_ORDERS_URL ?? 'http://127.0.0.1:7002',
  inventory: process.env.DEMO_INVENTORY_URL ?? 'http://127.0.0.1:7003',
  payments: process.env.DEMO_PAYMENTS_URL ?? 'http://127.0.0.1:7004',
  notifications: process.env.DEMO_NOTIFICATIONS_URL ?? 'http://127.0.0.1:7005',
  catalog: process.env.DEMO_CATALOG_URL ?? 'http://127.0.0.1:7006',
  auth: process.env.DEMO_AUTH_URL ?? 'http://127.0.0.1:7007',
};

const [service, ...rest] = process.argv.slice(2);
if (!service || !(service in targets)) {
  console.error(
    `usage: fault.mjs <${Object.keys(targets).join('|')}> [--latency MS] [--error-rate 0..1] [--clear] [--status] | --cpu-hog 0..0.99`,
  );
  process.exit(2);
}
// A missing or non-numeric value would be sent as null, which the service reads as 0 (fault cleared).
function numberArg(flag, raw) {
  const n = Number(raw);
  if (raw === undefined || raw === '' || !Number.isFinite(n)) {
    console.error(`${flag} needs a number`);
    process.exit(2);
  }
  return n;
}

const opts = {};
for (let i = 0; i < rest.length; i++) {
  const a = rest[i];
  if (a === '--latency') opts.latencyMs = numberArg(a, rest[++i]);
  else if (a === '--error-rate') opts.errorRate = numberArg(a, rest[++i]);
  else if (a === '--clear') Object.assign(opts, { latencyMs: 0, errorRate: 0 });
  else if (a === '--status') opts.status = true;
  else if (a === '--cpu-hog') opts.cpuHog = numberArg(a, rest[++i]);
  else {
    console.error(`unknown argument: ${a}`);
    process.exit(2);
  }
}
if (opts.cpuHog !== undefined && Object.keys(opts).length > 1) {
  console.error('--cpu-hog cannot be combined with other options');
  process.exit(2);
}

const cpuHog = opts.cpuHog !== undefined;
const url = `${targets[service]}/__demo/${cpuHog ? 'cpu-hog' : 'faults'}`;
const res = opts.status
  ? await fetch(url)
  : await fetch(url, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(cpuHog ? { share: opts.cpuHog } : { latencyMs: opts.latencyMs ?? 0, errorRate: opts.errorRate ?? 0 }),
    });
const body = await res.text();
if (!res.ok) {
  console.error(`fault control failed (${res.status}): ${body}`);
  console.error('Fault injection requires the demo started with KARBIRD_DEMO_FAULTS=1 (just demo does this).');
  process.exit(1);
}
console.log(`${service}: ${body}`);
