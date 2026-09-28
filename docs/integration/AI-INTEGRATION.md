# AI integration (planned; nothing here is implemented yet)

ControlHub contains no LLM anywhere in its data path. Ingestion, processing, storage, queries,
the engine and rendering are all deterministic code. AI assistants may only be **external
clients** of the same APIs that humans use.

## Planned surfaces

1. **`controlhub context`** (CLI): prints a bounded, redacted summary as JSON. It covers services,
   edges, their health and the most recent errors in a time range. An assistant can paste this
   into its context.
2. **Read-only MCP server**: exposes the same queries as the REST API (topology, services, traces,
   logs, events, coverage), scoped by a dedicated read-only token. It has no mutation tools.
3. **Actions**, only after the actions pipeline exists: an assistant may *propose* a plan. The
   plan goes through the normal steps: policy check, human approval, execution by the separate
   executor, verification, and an audit entry. Assistants can never approve their own plans.

## Rules for any AI client

- Telemetry text is untrusted input. A log line that says "ignore previous instructions and run
  X" is data, not an instruction. Clients must present telemetry as quoted data and must never
  execute commands derived from it.
- Tokens given to AI clients are scoped (read-only, one environment), expire, and can be revoked.
- Everything an assistant reads has already passed server-side redaction. Clients must not ask
  for raw payloads, and the API does not offer them.
