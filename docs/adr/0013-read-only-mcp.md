# ADR-0013: Read-only MCP server as a CLI subcommand over the REST API

## Decision
- The MCP server is `controlhub mcp`, a stdio server in the existing Rust CLI. It is not a
  separate network service, and the API itself does not speak MCP.
- It authenticates to the REST API with a read-only, environment-bound API token from
  `CONTROLHUB_API_TOKEN` (ADR-0011). All authorization stays in the API, so the MCP process adds no
  privileges and cannot widen scope. Tool arguments cannot name a project or environment.
- Only query tools exist, all annotated `readOnlyHint: true`. Proposing, approving or running
  actions is not exposed. Even if it were, the API rejects mutations authenticated with a bearer
  token.
- Results are recursively sanitized: strings are cleaned and truncated, arrays and nesting are
  bounded. Each result carries the untrusted-data notice. The `initialize` instructions state the
  same rule.
- The JSON-RPC handling is hand-written (about 150 lines: `initialize`, `ping`, `tools/list`,
  `tools/call`, notifications, error codes), with the message size capped at 1 MiB. We did not
  add an MCP SDK dependency for four methods on stdio; this involves no cryptography.

## Consequences
- Anyone who can run the CLI with a token gets exactly what that token can read over REST, and
  nothing more.
- There is no HTTP/SSE MCP transport. Remote MCP would need its own auth design (OAuth per the MCP
  spec) and is out of scope.

## Alternatives rejected
- MCP inside the API process: adds an unauthenticated-by-default protocol surface to the
  internet-facing service.
- A generic "call any GET endpoint" tool: makes the surface depend on every future route.
