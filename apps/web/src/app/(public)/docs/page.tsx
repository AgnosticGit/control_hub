import type { Metadata } from "next";
import { Prose } from "../prose";

export const metadata: Metadata = {
  title: "Documentation",
  description: "ControlHub architecture, data model, API and security overview.",
};

export default function DocsPage() {
  return (
    <Prose title="Documentation" lead="An overview; the full documentation lives in the repository under docs/.">
      <h2>Data flow</h2>
      <p>
        SDK or Collector → ingest (auth, validation, redaction) → Kafka → processor → ClickHouse → query API → Web
        Worker running the WebAssembly engine → map and panels.
      </p>
      <h2>Time model</h2>
      <p>
        All timestamps are Unix nanoseconds. They cross JSON boundaries as decimal strings and are handled as BigInt in
        the browser, so no precision is lost.
      </p>
      <h2>Provenance</h2>
      <p>
        Every relation is <em>observed</em> (seen in telemetry), <em>declared</em> (configured) or <em>inferred</em>{" "}
        (derived, with listed reasons). Inferred relations never carry an invented confidence score.
      </p>
      <h2>Completeness</h2>
      <p>
        Query responses report the processing watermark, unresolved processing errors and truncation, so the UI can
        say when data may be incomplete instead of implying certainty.
      </p>
      <h2>API</h2>
      <p>
        REST under <code>/api/v1</code>, described by an OpenAPI document generated from the Rust server
        (<code>schemas/generated/openapi.json</code>).
      </p>
    </Prose>
  );
}
