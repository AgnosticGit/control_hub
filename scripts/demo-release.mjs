#!/usr/bin/env node
// Demo-only releases through the demo orchestrator (`just demo` must be running):
//   just demo-release orders 0.2.0    deploy a version (0.2.0 ships a regression in order reads)
//   just demo-release orders          show the current version and the one a rollback returns to
// Rolling back is a controlled action (`demo.release.rollback`), proposed and approved in karbird.
import { CONTROL_PORT } from "./demo-control.mjs";

const base = process.env.DEMO_CONTROL_URL ?? `http://127.0.0.1:${CONTROL_PORT}`;
const [service, version, ...rest] = process.argv.slice(2);
if (!service || rest.length > 0 || !/^[a-z0-9-]{1,32}$/.test(service)) {
  console.error("usage: demo-release.mjs <service> [version]");
  process.exit(2);
}

const url = `${base}/releases/${service}`;
let res;
try {
  res = version
    ? await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ version }) })
    : await fetch(url);
} catch (e) {
  console.error(`demo orchestrator unreachable at ${base} (is \`just demo\` running?): ${e.message}`);
  process.exit(1);
}
const body = await res.text();
if (!res.ok) {
  console.error(`release failed (${res.status}): ${body}`);
  process.exit(1);
}
console.log(`${service}: ${body}`);
