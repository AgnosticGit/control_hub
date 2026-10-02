# Security model (current implementation)

## Authentication

- **Browser sessions** use the `controlhub_session` cookie: random, HttpOnly, `SameSite=Strict`,
  and `Secure` unless `CONTROLHUB_ENV=dev`. Only its SHA-256 hash is stored, in `sessions`.
  Sessions expire and are revoked on logout.
- **CSRF** protection is a double submit: the `controlhub_csrf` cookie must match the
  `x-csrf-token` header on every cookie-authenticated mutation. The server stores only the hash.
- **Origin allowlist** (`CONTROLHUB_ALLOWED_ORIGINS`): a mutation or login whose `Origin` header
  is not on the list gets 403.
- **Passwords** are hashed with Argon2id (the `argon2` crate). Login returns the same 401 for a
  wrong password and for an unknown user, and it is rate-limited (429).
- **Ingestion tokens** are random and scoped to one tenant, project, environment and set of
  signals. Only a SHA-256 hash plus a short display prefix is stored. Tokens support `expires_at`
  and `revoked_at`. Admins create, revoke and rotate them in the web app (workspace → Tokens,
  session plus CSRF only). Rotation keeps the old token valid for a grace period of at most 24 h.
  Ingest caches credentials for up to 30 s, which bounds how long revocation takes there.
- **API tokens** (`cha_…`, ADR-0011) are for the CLI and MCP. They are read-only (any mutation
  → 403), bound to one environment (other scopes → 404), expire within 90 days, and are stored as
  hashes. Their effective access is intersected with the creator's current membership.
- **WebSocket live channel**: session credentials require an allowed `Origin`. The credential is
  re-checked every 30 s, connections are limited by a semaphore, and scope comes from the
  principal only.

## Multi-tenancy

- The tenant, project and environment for ingested data come only from the token. Attributes in
  the payload that claim a different tenant are ignored; an integration test covers this.
- Every API query is filtered by the scope resolved from the session's memberships. Requests for
  another tenant's resources return 404, so the API does not reveal whether they exist.
  Integration tests check this in both directions.
- Each service has its own least-privilege database user in PostgreSQL and in ClickHouse
  (`controlhub_ingest`, `controlhub_processor`, `controlhub_api`, `controlhub_executor`,
  `controlhub_archiver`). The executor may only read and update `action_plans`, insert into
  `audit_log`, and read environments and users. Its ClickHouse user is SELECT-only.
- The archiver (ADR-0015) may only `SELECT` from ClickHouse `logs`, use the `archive_*` tables and
  read `projects`. In object storage, each service has its own S3 identity limited to the
  archive bucket. The API never writes or deletes objects; give it read-only credentials in
  production (locally SeaweedFS 3.97 needs `Write` for reads, see ADR-0015). The API reads segments
  only for the caller's scope and checks each object's SHA-256 against the catalog before
  decoding it.

## Controlled actions (ADR-0012)

- Only allowlisted, typed actions with bounded parameters exist. Nothing in a plan is a command,
  path, URL or container name, and telemetry never creates a plan.
- Policy (demo-only environments, one open plan per target, 30 plans per hour) is evaluated at
  proposal, at approval and right before execution. Denials are audited.
- Four-eyes approval: approver ≠ proposer, enforced in the API and by a database CHECK. Operators
  propose, admins approve or reject (rejecting requires a comment). API tokens can do neither.
- Every state change commits together with its audit row. If the audit write fails, the change
  fails.
- Only `controlhub-executor` touches the observed system. Its demo adapter calls a loopback-only
  endpoint and runs `docker` with fixed arguments (no shell) after checking the container's
  compose labels. Web, API and ingest never get the Docker socket. A plan whose executor dies is
  marked failed and never retried automatically.

## Sensitive data

- The SDK does not collect bodies, full SQL (`enhancedDatabaseReporting: false`), or the
  Authorization and Cookie headers.
- ingest applies redaction regardless of what the SDK did (`controlhub-domain/src/redaction.rs`):
  - Keys containing authorization, cookie, password, secret, token, api_key, credential or session
    are replaced with `[REDACTED]`.
  - Body and statement attributes (`db.statement`, `db.query.text`, `http.*.body`) are dropped.
  - `Bearer …` and `Basic …` fragments in free text are masked.
  - Values are truncated to fixed limits.
- Telemetry is untrusted. The UI renders it only as React text nodes; there is no
  `dangerouslySetInnerHTML`. Nothing ever executes telemetry content.

## Secrets and environments

- `just bootstrap` generates every credential randomly into `.controlhub/`, which is gitignored and
  written with mode 0600 where the OS supports it. There is no shared default admin.
- The web app has no `NEXT_PUBLIC_*` configuration. The API URL is server-side only.
- Dev mode (`CONTROLHUB_ENV=dev`) must be set explicitly, and the API logs a warning at startup.
  The default is production behaviour.
- Demo fault injection works only when `CONTROLHUB_DEMO_FAULTS=1`, and only from loopback.

## Headers

- Every response carries `X-Content-Type-Options`, `Referrer-Policy`, `X-Frame-Options: DENY` and
  `Permissions-Policy`.
- `/app/*` responses carry `X-Robots-Tag: noindex` and `Cache-Control: no-store`.
- **Content-Security-Policy** (`apps/web/src/lib/csp.ts`):
  - `/app/*` gets a strict per-request policy from `proxy.ts`:
    `script-src 'nonce-…' 'strict-dynamic' 'wasm-unsafe-eval'`, with no `unsafe-inline` or
    `unsafe-eval` for scripts. These pages are rendered dynamically so Next.js can apply the nonce.
  - Public static pages and assets get a baseline policy. It allows inline scripts, because static
    pages cannot carry a nonce, but restricts everything else: `object-src 'none'`,
    `frame-ancestors 'none'`, `form-action 'self'`, `connect-src 'self'`.
  - The Pixi renderer imports `pixi.js/unsafe-eval`, which removes its need for `eval`. The e2e
    test `csp.spec.ts` checks the headers and that the workspace runs with no violations.

## Not done yet

- TLS termination. Everything is plain HTTP on loopback in dev; production would need rustls or a
  proxy in front.
- A general SSRF policy. No outbound request takes a URL from user input or telemetry: the executor
  only calls demo URLs from its own configuration, which must be loopback. Any future adapter
  needs a real egress policy.
- Complete audit. Token and action mutations are audited, but logins are still audited on a
  best-effort basis (an audit write failure does not fail the login), and failed logins are not
  audited.
- User and membership management (users come only from `bootstrap`).
