// Preload entry point: `node --import @controlhub/sdk-node/register app.js`.
//
// Loading the SDK before application modules guarantees that `http` and `pg`
// are patched before first use. The ESM loader hook is registered so that
// ESM applications are instrumented as well as CommonJS ones.

import { register } from 'node:module';
import { startControlHub } from './index.js';

register('@opentelemetry/instrumentation/hook.mjs', import.meta.url);

const handle = startControlHub();

let shuttingDown = false;
for (const signal of ['SIGINT', 'SIGTERM']) {
  process.once(signal, () => {
    if (shuttingDown) return;
    shuttingDown = true;
    handle
      .shutdown()
      .catch((err) => console.error('controlhub: telemetry shutdown failed:', err?.message ?? err))
      .finally(() => process.exit(0));
  });
}
