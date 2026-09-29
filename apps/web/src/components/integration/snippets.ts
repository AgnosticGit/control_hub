// Copy-paste configuration for sending telemetry to one ControlHub environment.
// Pure functions: the page renders the text, it never evaluates or executes it.

export const TOKEN_PLACEHOLDER = "<your-ingestion-token>";

export interface SnippetInput {
  /** Base OTLP/HTTP endpoint, e.g. `https://example.test/otlp`. */
  endpoint: string;
  /** Ingestion secret, or null to show a placeholder. */
  token: string | null;
  serviceName: string;
}

/** OTLP endpoint behind the web app's `/otlp` proxy, on the same public address as the site. */
export function otlpEndpoint(origin: string): string {
  return `${origin.replace(/\/+$/, "")}/otlp`;
}

/** Service names are resource attributes and end up in shell/env files: keep them boring. */
export function safeServiceName(raw: string): string {
  const s = raw
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64);
  return s || "my-service";
}

const token = (i: SnippetInput) => i.token ?? TOKEN_PLACEHOLDER;

/** Standard OpenTelemetry SDK variables; work for any language with an OTLP/HTTP exporter. */
export function otelEnv(i: SnippetInput): string {
  return [
    `OTEL_SERVICE_NAME=${safeServiceName(i.serviceName)}`,
    `OTEL_EXPORTER_OTLP_ENDPOINT=${i.endpoint}`,
    "OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf",
    // Header values are percent-encoded in this variable (OpenTelemetry spec), hence %20.
    `OTEL_EXPORTER_OTLP_HEADERS=authorization=Bearer%20${token(i)}`,
    "OTEL_TRACES_EXPORTER=otlp",
    "OTEL_LOGS_EXPORTER=otlp",
    "OTEL_METRICS_EXPORTER=none",
  ].join("\n");
}

/** Node.js with the official OpenTelemetry zero-code instrumentation. */
export function nodeSteps(i: SnippetInput): string {
  return [
    "npm install --save @opentelemetry/api @opentelemetry/auto-instrumentations-node",
    "",
    "# .env (or your process manager / container environment)",
    otelEnv(i),
    "",
    "# start your app with the instrumentation loaded first",
    "node --import @opentelemetry/auto-instrumentations-node/register server.js",
  ].join("\n");
}

/** OpenTelemetry Collector that forwards what your services already send it. */
export function collectorConfig(i: SnippetInput): string {
  return [
    "receivers:",
    "  otlp:",
    "    protocols:",
    "      grpc: {}",
    "      http: {}",
    "processors:",
    "  batch: {}",
    "exporters:",
    "  otlphttp/controlhub:",
    `    endpoint: ${i.endpoint}`,
    "    encoding: proto",
    "    headers:",
    `      authorization: "Bearer ${token(i)}"`,
    "service:",
    "  pipelines:",
    "    traces:",
    "      receivers: [otlp]",
    "      processors: [batch]",
    "      exporters: [otlphttp/controlhub]",
    "    logs:",
    "      receivers: [otlp]",
    "      processors: [batch]",
    "      exporters: [otlphttp/controlhub]",
  ].join("\n");
}

/** One-shot check from a terminal: an empty protobuf export is a valid request. */
export function curlCheck(i: SnippetInput): string {
  return [
    `curl -i -X POST "${i.endpoint}/v1/traces" \\`,
    `  -H "authorization: Bearer ${token(i)}" \\`,
    '  -H "content-type: application/x-protobuf" \\',
    '  --data-binary ""',
  ].join("\n");
}
