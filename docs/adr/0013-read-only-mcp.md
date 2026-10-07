# ADR-0013: Read-only MCP server as a CLI subcommand over the REST API

## Decision
- The MCP server is `karbird mcp`, a stdio server in the existing Rust CLI. It is not a
  separate network service, and the API itself does not speak MCP.
- It authenticates to the REST API with a read-only, environment-bound API token from
  `KARBIRD_API_TOKEN` (ADR-0011). All authorization stays in the API, so the MCP process adds no
  privileges and cannot widen scope. Tool arguments cannot name a project or environment.
- Only query tools exist, all annotated `readOnlyHint: true`. The typed surface is: environment
  context; services and topology; traces, logs and events; action-plan listing; infrastructure
  snapshots and history; traffic for one dependency edge; telemetry timeline and coverage; and a
  composed incident snapshot. Proposing, approving or running actions is not exposed. Even if it
  were, the API rejects mutations authenticated with a bearer token.
- Range tools accept either a relative `minutes` window or an exact `from_ns` / `to_ns` pair of
  decimal nanosecond strings. The scope remains fixed by the token. `get_infrastructure` uses an
  optional `at_ns` point and the edge tool requires IDs returned by topology.
- Every successful result carries a relative Karbird UI path. It includes a full UI URL only when
  the local CLI was given an explicit `KARBIRD_WEB_URL`; it never infers a web origin from the API
  port or puts a token in the link.
- Results are recursively sanitized: strings are cleaned and truncated, arrays and nesting are
  bounded. Each result carries the untrusted-data notice. The `initialize` instructions state the
  same rule.
- The JSON-RPC handling is hand-written (about 150 lines: `initialize`, `ping`, `tools/list`,
  `tools/call`, notifications, error codes), with the message size capped at 1 MiB. We did not
  add an MCP SDK dependency for four methods on stdio; this involves no cryptography.

## Consequences
- Anyone who can run the CLI with a token gets exactly what that token can read over REST, and
  nothing more.
- The MCP adapter is typed rather than a generic REST gateway. It has no archive-log or archive
  catalog tool, even though an existing API timeline may report its normal server-side log source.
- There is no HTTP/SSE MCP transport. Remote MCP would need its own auth design (OAuth per the MCP
  spec) and is out of scope.

## Alternatives rejected
- MCP inside the API process: adds an unauthenticated-by-default protocol surface to the
  internet-facing service.
- A generic "call any GET endpoint" tool: makes the surface depend on every future route.
