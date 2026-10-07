# MCP observability expansion

## Intent

Give an external coding assistant a compact, read-only view of the infrastructure and request
flow of the single environment bound to its Karbird API token. It must support incident
investigation at an exact time, without expanding the token's authority or exposing archive
storage, actions, arbitrary queries, or raw telemetry.

## Scope

Included:

- infrastructure snapshots and infrastructure history;
- line traffic, including byte-rate coverage;
- event and telemetry investigation over an absolute interval;
- data-completeness inspection;
- a composed incident snapshot;
- UI deep links for every MCP response;
- CLI, docs, unit and integration coverage.

Excluded:

- archived-log search and archive catalog access;
- action proposal, approval, execution or cancellation;
- remote MCP transport, OAuth, project selection and organisation administration;
- arbitrary SQL, Docker, shell or generic REST tools.

## Constraints

- `karbird mcp` remains a local stdio server.
- `KARBIRD_API_TOKEN` remains read-only and environment-bound. Tool arguments never identify
  project, environment, tenant or organisation.
- All data continues to pass through the existing REST API authorization and server-side
  redaction.
- Every tool is annotated `readOnlyHint: true`, `destructiveHint: false` and `idempotentHint: true`.
- MCP outputs preserve the current recursive sanitization limits: untrusted telemetry text,
  strings capped at 300 characters, arrays capped at 50 items, object keys capped at 200 and
  object nesting capped at 8.
- Nanosecond times are decimal strings. JavaScript number timestamps are never accepted or emitted.

## API reuse

This work adds no telemetry query endpoint. MCP calls the existing routes with the token scope:

- `/infrastructure?at=`
- `/infrastructure/timeline?from=&to=&buckets=`
- `/edges/traffic?source=&target=&relation=&from=&to=&buckets=`
- `/timeline?from=&to=&buckets=`
- `/coverage?from=&to=`
- existing services, topology, traces, logs and events routes.

The CLI is deliberately a typed adapter, never a generic GET proxy. Each tool defines its own
allowlisted query parameters and response meaning.

## Time selection

### Relative windows

Existing tools retain `minutes` with a range of 1 to 1440 and a default of 15. This is suitable
for recent triage.

### Absolute windows

The tools which query a range accept `from_ns` and `to_ns` as decimal positive `i64` strings.
They must be supplied together. `minutes` cannot be combined with either absolute field.

The common parser rejects zero, negative, non-decimal, overflowing and inverted ranges before any
HTTP request. Endpoint-specific server range restrictions still apply, including the seven-day
limit for infrastructure history.

`get_infrastructure` accepts optional `at_ns`; without it, the CLI uses the current time.

## New MCP tools

### `get_infrastructure`

Arguments: `at_ns?`.

Returns the existing infrastructure snapshot: machines, their resource metrics, containers,
instances and replicas, internal/external/unknown resources, matching reasons and deterministic
noisy-neighbour hints. It distinguishes missing agent metrics from a genuine zero value.

### `get_infrastructure_timeline`

Arguments: `from_ns`, `to_ns`, `buckets?` (1–1000, default 200).

Returns CPU and memory tracks by machine plus derived container lifecycle events. The response
keeps the API's event ordering and `events_total` rather than inventing chronology in MCP.

### `get_edge_traffic`

Arguments: `source`, `target`, `relation`, `from_ns`, `to_ns`, `buckets?` (1–600, default 120).

`source`, `target` and `relation` are opaque edge identifiers returned by topology; each is
validated as a short single-line string. The response includes calls, request and response bytes
per bucket, average and peak bytes/second, bucket duration and size coverage. A `null` rate means
the source telemetry did not report declared content lengths; it must never be rewritten to zero.

### `get_timeline`

Arguments: `from_ns`, `to_ns`, `limit?` (1–1000, default 200 per signal).

Returns the existing service telemetry timeline for an incident window.

### `get_coverage`

Arguments: `from_ns`, `to_ns`.

Returns data completeness for traces, logs, events and relevant sources so an assistant can state
whether an empty result is trustworthy.

### `get_incident_snapshot`

Arguments: one time selector (`minutes` or `from_ns` plus `to_ns`) and optional `service_id`.

This is a composed, bounded read-only result. It calls services, topology, traces filtered to
errors, error logs, events, coverage and the infrastructure snapshot at the interval end. If a
service is supplied, it filters the service-scoped routes and retains only adjacent topology
edges. It does not issue edge-traffic calls for every edge; the caller explicitly selects a line
with `get_edge_traffic` to avoid unbounded fan-out.

The response contains no generated diagnosis. It reports facts and deterministic noisy-neighbour
hints returned by the API.

## Existing tools with exact time

`list_services`, `get_topology`, `search_traces`, `search_logs` and `list_events` use the common
time selector. `get_context` retains its existing relative `minutes` interface because it is a
legacy convenience command; `get_incident_snapshot` is the exact-time replacement.

Trace IDs, service IDs, log query and limits retain their existing validation. Per-result limits
remain 50 even where an API could return more rows.

## Deep links

Each successful tool result gains `ui_path`, a relative route containing the bound project,
environment, selected time range and applicable focus (`service`, `edge`, `machine` or `trace`).
Paths are percent encoded by the CLI and contain no token.

`ui_url` is additionally included when `KARBIRD_WEB_URL` is set to a valid HTTPS URL or a
loopback HTTP URL. It is the resolved absolute version of `ui_path`. Without this variable, MCP
still returns `ui_path`; the CLI never guesses that the direct API port also serves the web app.

`KARBIRD_WEB_URL` is documented as optional and is never printed with credentials.

## Error behaviour

- Invalid argument combinations fail as MCP tool errors before HTTP.
- API 401 remains the existing token-rejected error.
- API 400/404 errors are returned as bounded tool errors; the MCP server does not substitute a
  different scope or retry a different endpoint.
- A partial incident snapshot returns an error rather than a mixture of silently missing sections.
- Every successful structured result keeps the existing untrusted-data notice and token scope.

## Documentation

Update ADR-0013 and `docs/integration/AI-INTEGRATION.md` / `CLI.md` to list the expanded,
read-only surface and the optional web URL. Archive access remains explicitly absent.

## Test plan

### Rust unit tests

- valid and invalid absolute time parsing, including exclusive time selectors;
- tool schemas and unknown arguments;
- edge identifier validation;
- deep-link construction, encoding and optional web URL validation;
- the incident snapshot's bounded service filter and error propagation.

### Integration tests

Extend `tests/integration/mcp.test.mjs` to exercise the new tools against `just dev` and
`just demo`:

- infrastructure snapshot returns the demo machine/container model;
- infrastructure timeline returns ordered metrics/events for a bounded window;
- edge traffic returns coverage and preserves unknown byte rates as `null`;
- timeline and coverage are scope-bound;
- incident snapshot uses exact timestamps and includes a UI path;
- a token for the isolation environment cannot observe demo infrastructure or traffic;
- malformed timestamps, inverted windows and invalid edge arguments fail;
- the server exposes only read-only tools and never echoes the token.

The full change is verified with `just check`, `just test`, `just test-integration` and
`just test-e2e` because it changes API-facing, user-visible integration behavior.
