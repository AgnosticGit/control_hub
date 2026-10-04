import type { Metadata } from "next";
import { Prose } from "../prose";

export const metadata: Metadata = {
  title: "Features",
  description: "Service map, timeline replay, traces, logs and events in one place — built on OpenTelemetry.",
};

export default function FeaturesPage() {
  return (
    <Prose title="Features" lead="What works in the current development build, and what does not yet.">
      <h2>Available now</h2>
      <ul>
        <li>OTLP traces and logs ingestion (HTTP protobuf and gRPC unary Export), custom events API.</li>
        <li>Durable pipeline: ingestion acknowledges only after the broker confirms the write.</li>
        <li>Service map with observed calls and inferred dependencies (with reasons).</li>
        <li>Trace list and waterfall, correlated logs and events, virtualized log view.</li>
        <li>Timeline: live, paused and historical modes, seek, step between events, playback speeds.</li>
        <li>State reconstruction in a Rust engine compiled to WebAssembly, running in a Web Worker.</li>
        <li>Tenant/project/environment isolation, cookie sessions with CSRF protection.</li>
        <li>
          Infrastructure layer: machine and container metrics from the Karbird agent (OTLP gauges and sums), services grouped
          into the machines they run on, and a deterministic noisy-neighbour hint.
        </li>
      </ul>
      <h2>Not yet available</h2>
      <ul>
        <li>Application metrics: OTLP histograms and summaries are rejected with an explicit error, not dropped silently.</li>
        <li>Realtime WebSocket updates (the app polls in live mode).</li>
        <li>Actions (restart/stop of demo services), CLI and MCP integration.</li>
      </ul>
    </Prose>
  );
}
