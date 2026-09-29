# ADR-0011: Read-only API tokens and the WebSocket live channel

## API tokens (`cha_…`)
- Used by non-browser clients (CLI `context`/`verify`, MCP). They are sent as `Authorization: Bearer`.
- **Read-only.** Any mutating request authenticated with a bearer token is rejected with 403
  before routing. Token management and actions also require a session credential.
- **Bound to one environment.** Requests for any other scope return 404 (not 403), so a token
  does not reveal which other projects exist.
- **Effective access is an intersection.** A token grants viewer access to its environment only while
  its creator still has a membership there. Removing the creator removes the token's access.
- **Always expire** (1–90 days). Only a SHA-256 hash is stored (tokens are 256-bit random, so a
  slow KDF adds nothing). `last_used_at` is written at most once a minute.
- Created, revoked and rotated only by admins through a session with CSRF. Each change is audited
  (`token.create`, `token.revoke`, `token.rotate`).

## Ingestion token rotation
- Rotation issues a new secret with the same name and scope. The old token gets
  `expires_at = now + grace` (at most 24 h) so senders can switch over without dropping data.
- Ingest caches credentials for up to 30 s. That is the documented upper bound for revocation to
  take effect there. The API checks tokens on every request.

## Live channel (`GET …/live`, WebSocket)
- The Worker, not React, owns the socket. Batches go straight into the WASM engine
  (incremental append). React only sees engine state snapshots.
- For session credentials, an `Origin` header from the allowlist is required, because the browser
  attaches cookies to cross-site WebSocket handshakes. Bearer tokens need no Origin.
- The credential is re-checked every 30 s. The socket closes with `authorization ended` when the
  session is logged out or the token is revoked.
- Server-side limits: a global connection semaphore, per-connection batch caps, and scope from the
  principal only. No client ever receives the tenant-wide stream.
- Protocol: `hello` / `batch` / `gap` / `heartbeat` / `error`. On `gap` the Worker reloads the
  window over HTTP. It reconnects with backoff and, after 3 failed attempts, falls back to HTTP
  polling (shown as "Live: polling").

## Alternatives rejected
- Server-Sent Events: fine for one-way traffic, but a WebSocket leaves room for the client to
  adjust its subscription without reconnecting, and both need the same auth handling.
- Long-lived unscoped tokens: an unbounded blast radius and hard to audit.
