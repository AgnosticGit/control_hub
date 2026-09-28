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
  and `revoked_at`. `just bootstrap` rotates the local tokens by revoking the old ones and issuing
  new ones.
  - Not implemented yet: an API or CLI for managing tokens, and rotation with an overlap window.

## Multi-tenancy

- The tenant, project and environment for ingested data come only from the token. Attributes in
  the payload that claim a different tenant are ignored; an integration test covers this.
- Every API query is filtered by the scope resolved from the session's memberships. Requests for
  another tenant's resources return 404, so the API does not reveal whether they exist.
  Integration tests check this in both directions.
- Each service has its own least-privilege database user in PostgreSQL and in ClickHouse
  (`controlhub_ingest`, `controlhub_processor`, `controlhub_api`).

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

## Not done yet

- A strict Content-Security-Policy with nonces (Next.js inline scripts need one; WASM needs
  `wasm-unsafe-eval`).
- TLS termination. Everything is plain HTTP on loopback in dev; production would need rustls or a
  proxy in front.
- An SSRF policy. There are no outbound fetches driven by user input yet; it will be needed for
  actions and integrations.
- Audit entries for all mutations. The `audit_log` table and endpoint exist, but only successful
  logins are written, on a best-effort basis: an audit write failure does not fail the login.
- The actions executor, which will be the only component allowed to reach the Docker socket.
  Neither web, API nor ingest mounts the socket.
