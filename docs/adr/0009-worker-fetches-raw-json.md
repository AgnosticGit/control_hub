# ADR-0009: The Worker fetches JSON and hands raw text to WASM

## Context
A timeline response can be tens of MB. Parsing it on the UI thread and then structured-cloning it
into the Worker would cost the parse plus a full copy.

## Decision
- The UI sends the Worker a same-origin URL. The Worker `fetch`es it (response capped at 64 MB)
  and passes the text to `applyBatch` / `loadTopology`, where serde parses it once, inside WASM.
- The Worker reads a few metadata fields (`from_ns`, `to_ns`, watermark, completeness) by looking
  for `],"from_ns":` at the end of the response, so it does not parse the whole document twice.

## Consequences
- This depends on the API serializing `items` first. The API response struct keeps that field
  order, and the integration tests read these fields.
- **Known limitation:** this is fragile. The planned replacement is a small binary or NDJSON
  framing, or response headers carrying the metadata.
