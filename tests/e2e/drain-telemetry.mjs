import { randomUUID } from "node:crypto";
import { loadEnvFile } from "../../scripts/lib.mjs";
import { ingestEvent, ingestSpans, randomHex } from "./specs/helpers.ts";

// Ingest ACK means Kafka has the record, not that ClickHouse has it. A marker in
// each topic, followed by a ClickHouse read, fences all earlier records for this
// scope before the next spec clears its rows.
const infra = loadEnvFile(".karbird/infra.env");
const suffix = randomUUID().replaceAll("-", "");
const service = `e2e-drain-${suffix}`;
const now = BigInt(Date.now()) * 1_000_000n;
await ingestSpans(service, [{
  traceId: randomHex(16),
  spanId: randomHex(8),
  name: "e2e processor drain marker",
  kind: 2,
  startNs: now,
  endNs: now + 1_000_000n,
}]);
const eventId = await ingestEvent(service, "e2e.processor.drain", now);

const port = infra.CH_CLICKHOUSE_HTTP_PORT;
const user = infra.CH_CH_ADMIN_USER;
const password = infra.CH_CH_ADMIN_PASSWORD;
if (!port || !user || !password) throw new Error("missing ClickHouse admin connection for E2E drain barrier");
const query = `SELECT concat(toString((SELECT count() FROM karbird.spans WHERE tenant_id='isolation-tenant' AND project_id='isolation' AND environment_id='isolation-test' AND service_name='${service}')), ',', toString((SELECT count() FROM karbird.events WHERE tenant_id='isolation-tenant' AND project_id='isolation' AND environment_id='isolation-test' AND event_id='${eventId}'))) FORMAT TabSeparated`;
const endpoint = `http://127.0.0.1:${port}/`;
const auth = `Basic ${Buffer.from(`${user}:${password}`).toString("base64")}`;
const deadline = Date.now() + 30_000;
let drained = false;
while (Date.now() < deadline) {
  const response = await fetch(endpoint, { method: "POST", headers: { authorization: auth }, body: query });
  if (!response.ok) throw new Error(`ClickHouse drain barrier returned ${response.status}`);
  if ((await response.text()).trim() === "1,1") {
    drained = true;
    break;
  }
  await new Promise((resolve) => setTimeout(resolve, 250));
}
if (!drained) throw new Error("timed out waiting for the E2E telemetry processor drain barrier");
