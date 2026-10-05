# ADR-0019: Link statistics from the engine, data flow from declared HTTP body sizes

## Context
A line on the System Map stands for calls between two components, but clicking it did nothing:
the call count, errors and latency of one link were only visible as the line's colour and width,
and nothing said how much data flows over it. Spans already exist for every call. OpenTelemetry's
HTTP instrumentation records no body sizes, and bodies themselves must never be collected.

## Decision
- **Clicking a line opens the link in the inspector** (also reachable from every relation of a
  component's inspector, since the canvas is not in the accessibility tree). The renderer
  hit-tests the drawn polylines (7 screen px) after cards and chips and before machine frames.
  A link is identified by the node IDs at its ends, not the engine edge index, so it survives a
  relayout.
- **Calls, errors and latency come from the engine** (`Engine::edge_metrics`), over the same
  30 s window and the same counters that colour the line: calls started in the window, errors and
  durations of calls that ended in it (avg, nearest-rank p50/p95, max), calls in flight, and a
  15-slice sparkline. A span belongs to the link exactly as in the timeline build (`span_edge`):
  the callee's server span for a service call, the caller's client span for a database, queue or
  external host, the root server span for requests from outside.
- **Data flow comes from the API** (`GET …/edges/traffic?source&target&relation`), over the
  loaded range in up to 600 buckets: calls, request and response bytes per bucket, the average
  bytes per second over the range and the busiest bucket (peak) per direction. The same three
  edge kinds select the spans; for a service call the callee's sizes win and the caller's fill in.
- **Sizes are only the declared `Content-Length`.** The Node SDK sets `http.request.body.size`
  and `http.response.body.size` from it on both client and server spans (non-negative integers
  only); chunked messages without the header stay unknown. Redaction keeps these keys and still
  drops `http.request.body` / `http.response.body`.
- **Unknown is not zero.** Calls without a size are counted but add no bytes; the response says
  how many calls declared one (`total_sized_calls`, `size_coverage`), byte rates are `null` when
  none did, and the UI says the services do not report sizes instead of showing `0 B/s`.

## Consequences
- Database and queue links have no data flow (their client spans carry no sizes); HTTP links from
  services instrumented with `@karbird/sdk-node` do. Other SDKs need the same two attributes.
- Rates are a lower bound when coverage is partial; the coverage is always shown next to them.
- The peak is per bucket of the requested range, so it flattens as the range grows; the bucket
  length (`bucket_ns`) is part of the response.
