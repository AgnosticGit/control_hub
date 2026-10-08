# Persisted server topology and stable System Map geometry

## Outcome

Karbird builds and persists service relationships and shared map geometry on the backend.
Opening the workspace reads a saved graph. Repeated refreshes with unchanged structure do not
rebuild layout or reset the camera. All users of the same environment receive the same base
geometry. Infrastructure mode preserves the same guarantee for machine frames and replicas.

This implements the user's request to move relationships to the backend and save them so users
do not repeatedly wait for reconstruction. It includes automatic placement, not manual map editing.

## Current behavior and scope

The topology REST handler already derives observed service calls and inferred dependencies from
ClickHouse. It repeats aggregation for every request. WASM can also discover relationships from
timeline items. `Engine::load_topology` replaces indices and relayouts, even on unchanged snapshots.
Live clients request topology every ten seconds. Pixi reroutes lines on topology updates and
uses temporary fallback paths when frame geometry precedes the topology view.

The canonical graph, base coordinates, corridor waypoints, machine frames and replica placements
become server-owned. WASM continues timeline reduction, metrics, picking, filters and render-buffer
production; Pixi continues drawing and screen-dependent line styling. Browser discovery may request
a graph refresh but must not create an independently authoritative graph or rearrange canonical nodes.

Open clients automatically adopt a changed backend revision, including revised placement. Apply
graph membership and geometry together; retain camera and selection when the selected entity exists.

## Storage and identity

PostgreSQL stores a latest graph snapshot per authenticated tenant/project/environment and layout
version. Nodes use stable entity IDs; edges use source, target and relation as their identity.
Snapshots carry a structure revision, schema/layout version, updated timestamp and source watermark.
The saved document includes node coordinates, edge corridor points and infrastructure placements.
Metrics, activity, CPU and error rates do not change the structure revision.

Live graph inventory retains previously observed entities and edges with first/last seen timestamps.
Inactivity is represented explicitly rather than silently deleting relationships at the rolling-window
boundary. The render layer continues showing activity for the selected time; inventory presence alone
does not imply that a call occurred in that time window.

Historical requests retain exact range semantics. Their membership comes from telemetry in that
range and uses shared placement where possible. Historical results must not contaminate live inventory.
Historical snapshot caching is bounded and keyed by scope and exact range, with algorithm validation
on read and ten-second revalidation. Existing processor watermarks are global rather than scoped,
so revalidation detects late telemetry without using a global watermark as a scope cache key.
Historical ranges and selected-time placement variants share 64 cache entries per environment.
Placement variants are keyed by structural membership and ordinary geometry; CPU and cursor time
do not cause a fresh layout. The selected infrastructure metrics travel with the matching geometry.

## Graph builder and refresh lifecycle

A backend graph builder reuses existing relationship evidence and native Rust layout code. It runs
for active scopes independently of individual browser polling. Only one builder owns a scope at a
time, using a PostgreSQL lease/advisory lock so multiple API replicas do not duplicate expensive work.
Refresh cadence is initially ten seconds; idle scopes do not perform continuous aggregation.

Reads authorize access before fetching a snapshot. Existing snapshots are returned immediately while
refresh happens in the background. First access without a snapshot performs a bounded initial build;
this cold-start cost is explicit. Publish a complete revision atomically after successful computation.
Failure retains the last valid snapshot, reports freshness and logs the failed refresh.
Refresh queues, snapshot size and historical cache size have explicit bounds.

Canonical fingerprints sort nodes and edges by stable IDs and include structural resource/host
membership. A changed metric does not trigger layout computation. An identical graph reuses its
revision and geometry. New nodes are placed around existing saved coordinates; existing cards stay
anchored. The router recomputes affected corridors with obstacle/overlap checks. Topology changes
must not automatically refit a user's camera after the first fit.

## API and browser contract

Extend the existing scoped topology response with versioned saved geometry, revision and freshness.
Keep calls/errors and completeness explicit for the requested time range. Provide conditional reads
through ETag/If-None-Match for unchanged serialized responses, including selected-time metrics. Authentication and telemetry-view
permissions apply identically to normal and conditional responses.

Worker loads the saved geometry into WASM. A matching revision updates time-dependent evidence
without replacing node indices, clearing reducer caches or relayouting. Frame and topology geometry
are consumed as one compatible revision, avoiding temporary fallback routes between revisions.
Filter changes hide/dim entities without relocating the canonical graph. Infrastructure changes
update server-provided placements; stale/missing agent data is represented as unknown.

MCP receives the same canonical relationships through the existing topology API. Public demo data
and standalone engine tests retain an explicit local-layout path; the authenticated workspace uses
server geometry. Generated OpenAPI, TS and WASM artifacts are regenerated together.

## Verification

- Rust unit/property tests: stable fingerprint across ordering; identical refresh retains revision;
  metric-only changes retain geometry; additions retain existing positions; valid nonoverlapping
  corridors; canonical graph not mutated by client timeline batches.
- Storage/integration tests: persisted snapshot survives API restart; scopes isolated; single builder
  across concurrent requests; failed refresh preserves snapshot; history and late data semantics;
  conditional reads still require authorization.
- Browser tests: reopening uses identical coordinates; repeated live refresh retains camera/positions;
  Infrastructure replicas appear on each machine; filters and resize do not rebuild geometry;
  new service appears without resetting existing cards; selected service/edge survives revision.
- Required checks: codegen, WASM rebuild, just check, affected suites, integration and E2E. Report any
  environmental blockers or unrelated failures explicitly. No commits, pushes or deployments.

## Alternatives

Caching only the current JSON response reduces database queries but leaves repeated browser layout
and split graph ownership. Persisting only browser coordinates does not provide a shared graph for
API/MCP or coordinated updates. The selected approach persists backend graph and geometry together.
