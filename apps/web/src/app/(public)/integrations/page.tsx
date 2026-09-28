import type { Metadata } from "next";
import { Prose } from "../prose";

export const metadata: Metadata = {
  title: "Integrations",
  description: "Connect Node.js services and any OpenTelemetry-compatible SDK or Collector to ControlHub.",
};

export default function IntegrationsPage() {
  return (
    <Prose title="Integrations" lead="ControlHub speaks standard OTLP. Anything that exports OTLP can send data.">
      <h2>Node.js HTTP services</h2>
      <p>
        The <code>@controlhub/sdk-node</code> package wraps the official OpenTelemetry SDK: HTTP and PostgreSQL
        instrumentation, trace-correlated logs and request events. Load it before your application code:
      </p>
      <pre>{`CONTROLHUB_SERVICE_NAME=orders \\
CONTROLHUB_INGEST_TOKEN=<scoped token> \\
node --import @controlhub/sdk-node/register server.js`}</pre>
      <p>Headers, request/response bodies and full SQL text are not recorded.</p>
      <h2>OpenTelemetry SDKs and Collector</h2>
      <p>
        Point an OTLP exporter at <code>/v1/traces</code> and <code>/v1/logs</code> (HTTP, protobuf) or the gRPC
        endpoint, with header <code>Authorization: Bearer &lt;token&gt;</code>. OTLP/JSON is not supported yet and is
        rejected with HTTP 415.
      </p>
      <h2>Custom events</h2>
      <p>
        <code>POST /api/v1/events/batch</code> accepts deployment, configuration and request lifecycle events with
        nanosecond timestamps encoded as decimal strings.
      </p>
    </Prose>
  );
}
