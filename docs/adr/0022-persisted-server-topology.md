# ADR-0022: Persisted server topology and shared map geometry

## Context

Topology used to aggregate ClickHouse on each read, then replace browser engine indices and
recompute layout. A ten-second poll could consequently recreate routes even when the graph had
not changed. Browser timeline inference could also produce a different graph from API clients.

## Decision

- The Rust API aggregates relationship evidence and computes both ordinary and infrastructure
  geometry natively with `karbird-engine-core`. PostgreSQL `topology_snapshots` stores the complete
  graph document and geometry, scoped by authenticated tenant/project/environment.
- `GET …/topology?live=true` reads one durable live inventory. Previously observed entities/edges
  remain represented when inactive; retained edges have zero calls/errors for the current window
  and preserve `last_seen_ns`. Queries without `live=true` retain exact range membership.
- A structural fingerprint includes graph identity/kind/provenance, host membership, replica
  membership, resource placement and layout algorithm version. CPU, RAM and call counters do not
  change geometry revision. First load has a bounded initial build; subsequent reads return saved
  data immediately and refresh stale data in the background.
- Active live scopes refresh every ten seconds, stop after two minutes without reads, and resume
  on access. A process admits at most 64 live watches and two simultaneous builds. PostgreSQL
  watch leases coordinate replicas; a separate 60-second fenced build lease protects publication.
  Background refresh has a timeout and preserves the last valid document on failure.
- Historical ranges and placement variants share a cache of at most 64 completed/expired entries per environment; ranges revalidate
  after ten seconds, including late telemetry. Responses retain the actual stored `from_ns/to_ns`
  and expose `updated_at_ns`, completeness and geometry revision. ETag covers the authorized scope and the full serialized response, including selected-time metrics,
  not just geometry; conditional reads still authorize the caller before returning 304.
- Inventory is bounded at 2,000 nodes / 10,000 edges and stored documents at 16 MiB. Limits are
  explicitly reported through completeness and warning logs rather than silently represented
  as a complete graph. No underlying telemetry is deleted by map caching.
- Existing ordinary-map cards and corridor points stay anchored across additions. New cards use
  a free slot beside an existing caller without colliding with cards/corridors. Changes in host
  or replica membership rebuild the infrastructure placement as a shared server revision to keep
  machine bands and replica frames valid. Metric changes update labels and noisy-neighbour hints
  from the selected infrastructure snapshot without recomputing placement.
- Infrastructure requests with a range and selected `at` return the snapshot and matching server
  geometry together. Placement variants are keyed by structural membership and the ordinary-map
  geometry, not cursor time or CPU values. Revisiting that placement reuses the saved layout;
  metric changes still invalidate the response ETag. Ordinary-map reads do not require a separate
  selected-time infrastructure query. Layout jobs share two CPU permits, retained until a blocking
  job finishes even if its waiting request times out.
  Placement identities exclude the live geometry revision when its ordinary coordinates are
  unchanged; unrelated live machine changes therefore do not rebuild a fixed historical placement.
- WASM validates and consumes the saved geometry, reuses unchanged snapshots, and stops discovering
  canonical nodes/edges from timeline items. Private workspaces explicitly enable server ownership
  before their first snapshot, preventing local discovery during startup. Unknown items stay stored until a later server graph
  can attribute them. Local layout/inference remains available for standalone engine users and demos.
- Worker supplies topology metadata together with the matching frame. Stale topology loads are
  ignored; open clients apply changed geometry automatically on their next poll. Pixi does not
  automatically fit again on a graph/layer update; camera and stable-ID selections are preserved.
  Polling continues for paused, replay and historical views, preserving the selected cursor.
  Selected-time infrastructure reads also revalidate while paused, without overlapping requests
  or publishing responses for an obsolete cursor. Table view disables camera culling and resolves
  row indices against its matching frame metadata, so off-camera saved cards remain accessible.
- Renderer fit supports large persisted inventories at smaller scales. Inferred and noisy-neighbour
  dashes use screen spacing and a per-path segment bound; very long saved corridors do not allocate
  an unbounded number of invisible world-space strokes.
  Fit uses visible edge corridors and preserves fractional overlay measurements. Opaque machine
  headers take click priority over a nearby edge's screen-scaled hit radius.

## Operations

Run the ordinary deployment migration step before starting the new API. It creates the snapshot
table and grants the API role access only to that table. Restarting an API or opening another
browser reuses PostgreSQL documents. Only the first cold build must wait for aggregation/layout.
No new deployment secrets or infrastructure service are required.

Refresh logs distinguish timeout, aggregation failure, failed publication and lease release failure.
To inspect freshness, compare `updated_at_ns` to the current time and inspect `geometry.revision`.
Failed/expired refreshes are retried on access or by the active watch; they never replace the last
valid map with an empty response. Algorithm-version changes invalidate saved geometry on read.

## Consequences

Live structure is eventually consistent at the refresh/poll cadence. Activity and counters still
follow timeline data. Saved inventory presence means an entity was observed, not that it is currently
running or communicating. Historical cache is refreshed even for old ranges so late ingestion is
not permanently hidden. Manual map editing, pinned team layouts and an inventory-deletion UI remain
separate features.
