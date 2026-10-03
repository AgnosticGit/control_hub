# ADR-0017: Prebuilt images, two-host Compose deploy from GitHub Actions

## Context
The first production target is two small VPS (1/8 OCPU, 1 GB RAM, x86_64). Building Rust, WASM and
Next.js on such a host is impossible in practice (minutes of CPU per crate, not enough memory for
the linker), and copying the monorepo to every server couples deploys to the source tree. The dev
`bootstrap` seeds demo data, rotates ingest tokens and writes credential files — none of which may
happen on a production deploy.

## Decision
- **Images are built once, in CI** (`deploy/Dockerfile`, `deploy/docker-bake.hcl`): one Rust
  stage compiles all five service binaries, a WASM stage builds the engine, the web stage builds
  Next.js with `output: "standalone"`. Six images go to GHCR as
  `ghcr.io/<owner>/controlhub-<service>:<tag>`; the tag is the git tag (`v*`) or `sha-<12>`,
  never `latest`. Hosts only pull.
- **Two hosts, two Compose files.** `storage` runs PostgreSQL, ClickHouse and SeaweedFS with
  ports bound to its private IP only. `app` runs Kafka, ingest, processor, API, executor,
  archiver, web and Caddy; only Caddy (80/443) is public. Caddy terminates TLS (ACME) and routes
  `/api/v1/*` to the API, `/otlp/*` to ingest (prefix stripped), everything else to web.
- **One deploy secret.** Every service password and S3 key is derived from
  `CONTROLHUB_SECRET_SEED` with HMAC-SHA256 per purpose (`scripts/deploy-config.mjs`), so GitHub
  holds one secret and every deploy renders identical values. The rendered files exist only on
  the runner and in `/opt/controlhub` on the hosts (mode 600).
- **`controlhub-api migrate` replaces `bootstrap` on servers**: migrations, built-in roles,
  service roles and ClickHouse users, retention (ADR-0016, `--shrink-retention` to confirm a
  decrease), and the owner account if it does not exist. It seeds nothing, rotates nothing,
  writes no files and never resets an existing owner's password. It runs on every deploy before
  the app host is updated.
- **Workflow** (`.github/workflows/deploy.yml`): tag `v*` or manual run → the full CI workflow →
  build and push → SSH (pinned `known_hosts`) → storage up + migrate → app up → public smoke test.
  Hosts log in to GHCR with the job's `GITHUB_TOKEN` and log out afterwards.

## Consequences
- A release is `git tag vX.Y.Z && git push --tags`; a rollback is a manual run on the older tag
  (migrations must stay backward compatible for one release).
- Changing `CONTROLHUB_SECRET_SEED` after the first deploy breaks the PostgreSQL admin login
  (set once when the volume is created); rotating it needs a manual password change.
- Memory is not tuned yet: with defaults each host idles around 0.5 GB, so `deploy/host-setup.sh`
  adds 2 GB of swap. Limits for 1 GB hosts are a separate task.
- Traffic between the hosts (PostgreSQL, ClickHouse HTTP, S3) is plaintext on the private
  network; restricting it to the app host's private IP in the cloud firewall is mandatory.
