# ADR-0007: Browser authentication

## Decision
- The session is an opaque random token in an HttpOnly, `SameSite=Strict` cookie (`Secure` except
  in explicit dev mode). Only its SHA-256 hash is stored in PostgreSQL, so revocation is a single
  row update.
- CSRF uses the double-submit pattern: a JS-readable `karbird_csrf` cookie must match the
  `x-csrf-token` header on mutations. Only its hash is stored and it is bound to the session.
- Origin allowlist: a mutation or login carrying a foreign `Origin` header is rejected with 403.
- Passwords use Argon2id from the audited `argon2` crate. No custom cryptography.
- The Next.js `proxy.ts` only checks that the cookie exists, to redirect early. Real authorization
  always happens in the Rust API.

## Alternatives rejected
- JWT in localStorage: it can be read by any XSS, and revoking it is hard.
- Server-rendered pages calling the API with forwarded cookies: more complexity for no benefit at
  this stage.
