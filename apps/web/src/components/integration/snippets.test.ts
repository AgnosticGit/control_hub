import { describe, expect, it } from "vitest";
import { TOKEN_PLACEHOLDER, collectorConfig, curlCheck, nodeSteps, otelEnv, otlpEndpoint, safeServiceName } from "./snippets";

const input = { endpoint: "https://ch.example/otlp", token: "chi_secret", serviceName: "Orders API" };

function envMap(text: string) {
  return Object.fromEntries(
    text
      .split("\n")
      .filter((l) => /^[A-Z_]+=/.test(l))
      .map((l) => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1)]),
  );
}

describe("integration snippets", () => {
  it("points OTLP at the site's /otlp proxy", () => {
    expect(otlpEndpoint("https://slit-dandy-scallion.ngrok-free.dev")).toBe("https://slit-dandy-scallion.ngrok-free.dev/otlp");
    expect(otlpEndpoint("http://localhost:3000/")).toBe("http://localhost:3000/otlp");
  });

  it("configures a standard OpenTelemetry SDK for protobuf traces and logs only", () => {
    expect(envMap(otelEnv(input))).toEqual({
      OTEL_SERVICE_NAME: "orders-api",
      OTEL_EXPORTER_OTLP_ENDPOINT: "https://ch.example/otlp",
      // ControlHub ingest accepts only protobuf and has no metrics pipeline.
      OTEL_EXPORTER_OTLP_PROTOCOL: "http/protobuf",
      OTEL_EXPORTER_OTLP_HEADERS: "authorization=Bearer%20chi_secret",
      OTEL_TRACES_EXPORTER: "otlp",
      OTEL_LOGS_EXPORTER: "otlp",
      OTEL_METRICS_EXPORTER: "none",
    });
  });

  it("uses a placeholder until a token has been created", () => {
    const none = { ...input, token: null };
    for (const text of [otelEnv(none), nodeSteps(none), collectorConfig(none), curlCheck(none)]) {
      expect(text).toContain(TOKEN_PLACEHOLDER);
      expect(text).not.toContain("chi_secret");
    }
  });

  it("puts the token and endpoint into every snippet", () => {
    for (const text of [nodeSteps(input), collectorConfig(input), curlCheck(input)]) {
      expect(text).toContain("chi_secret");
      expect(text).toContain("https://ch.example/otlp");
    }
    expect(nodeSteps(input)).toContain("node --import @opentelemetry/auto-instrumentations-node/register");
    expect(collectorConfig(input)).toContain("encoding: proto");
    expect(curlCheck(input)).toContain('"https://ch.example/otlp/v1/traces"');
  });

  it("sanitises service names so they cannot break the env file", () => {
    expect(safeServiceName("  Payments Service ")).toBe("payments-service");
    expect(safeServiceName("a\nOTEL_EXPORTER_OTLP_ENDPOINT=https://evil")).toBe("a-otel_exporter_otlp_endpoint-https-evil");
    expect(safeServiceName("$(rm -rf /)")).toBe("rm--rf");
    expect(safeServiceName("")).toBe("my-service");
    expect(safeServiceName("x".repeat(100))).toHaveLength(64);
    expect(otelEnv({ ...input, serviceName: "a\nB=1" }).split("\n")).toHaveLength(7);
  });
});
