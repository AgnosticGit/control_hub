# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

karbird serves the people who run and change distributed systems, often the same small group wearing several hats:

- **On-call / SRE engineers** in the middle of an incident: something is red now, they need to see which service, since when, and rewind to the moment before it broke.
- **Backend developers** trying to understand how their services actually talk to each other and debugging a request across service boundaries.
- **Small product teams without a dedicated SRE**, where one workspace covers observing, investigating and (carefully) acting on the system.
- **Developers working with an AI coding assistant**, which gets read-only, scoped context about the running system through the `karbird` CLI and MCP server.

Roles inside a project are Owner, Admin, Operator and Viewer (plus custom role templates); karbird-wide administration has Super Admin, Administrator and Auditor.

## Product Purpose

karbird is an observability, history-exploration and controlled-management platform for distributed systems. It ingests standard OpenTelemetry traces, logs and events, keeps their history in ClickHouse, and shows the system as a live service map with a time machine: pause, scrub and replay what the system was doing, rebuilt deterministically from recorded data. Later it lets teams change the system through a controlled, audited action flow.

Success: a user who opens karbird during a problem can tell what is happening, what was happening a few minutes ago, and why — without switching tools — and any change they make to the system is planned, approved, verified and audited.

## Positioning

Four things together, which neighboring tools do not combine truthfully:

1. **Live map plus time machine.** State of the whole system at any moment is reconstructed deterministically (Rust engine compiled to WebAssembly) and replayed on the map and timeline, not just charted.
2. **Controlled actions.** Changes go plan → policy → four-eyes approval → execution by a separate executor → verification from telemetry → audit trail. Never free-form commands.
3. **Safe context for AI assistants.** A read-only CLI/MCP surface over real telemetry, scoped by tenant/project/environment.
4. **Honest data.** Every map edge says whether it was observed or inferred and why; completeness and gaps are shown explicitly, never smoothed over.

## Operating Context

- Users sign in to the private `/app` workspace (cookie sessions, CSRF); public pages (landing, features, docs, getting started, integrations) explain and onboard.
- Workspace is scoped to a project and environment: Overview (service map, inspector, timeline, traces/logs/events), Actions, Integration (tokens, OTel env vars, connection status), Tokens, Settings (members & roles), plus personal Account, onboarding and karbird Admin.
- Typical scenes: an incident where a node on the map turns amber/red and the user scrubs the timeline back; connecting a new service with an ingest token; proposing and approving an action as two different people; an assistant pulling context via CLI/MCP.
- **Delivery target:** hosted SaaS. The repository includes a two-host production deployment path; it does not document a verified live production instance. Public pages must not present the local development demo as the customer product. (Development itself runs on a local demo stand with a 7-service demo system and fault injection; that is internal, not customer-facing.)

## Capabilities and Constraints

- **Available:** OTLP traces and logs (HTTP protobuf and gRPC), custom events API, machine and container gauges and sums from the Karbird Collector; durable pipeline (ingest ACKs only after the broker ACK); service map with observed and inferred dependencies, external systems; trace list and waterfall, correlated logs and events; timeline with live/paused/historical modes, seek, step, replay range, speeds; WebSocket live updates; API tokens (create/rotate/revoke); self-service sign-up, organization email invitations, project invite links, onboarding, personal account; per-project roles and permissions; karbird admin panel; controlled actions with the demo adapter only; `karbird` CLI and read-only MCP server; log archiving and per-signal retention; production deployment tooling.
- **Not available yet:** application OTLP histograms and summaries, archiving spans and events, a verified live production deployment, real (non-demo) action adapters, email verification/password reset, notifications for pending approvals.
- **Hard constraints:** telemetry text is untrusted — rendered as text only, never as HTML; every query is scoped by the authenticated principal's tenant/project/environment; secrets are redacted at ingestion; the map must have a working table fallback when WebGL is unavailable; `/app` is private (noindex, no-store) with strict nonce CSP.
- **Terminology:** tenant, project, environment; service map, node, edge (observed / inferred), external clients/services; timeline, live / paused / historical, replay range, completeness; trace, span, log, event; action plan, policy check, approval (four-eyes), executor, verification, audit trail; ingest token, API token.
- **Open decisions:** pricing, plans and licensing for the SaaS; supported production scale (no published load numbers yet).

## Brand Commitments

- Name: **karbird**. Public tagline in use: "Observability · Replay · Control"; headline "See what your system is doing — and what it was doing five minutes ago."
- Voice: plain, precise and honest about limits — the Features page openly lists what does not work yet. Brand marks, app icon and owl artwork are in `docs/brand/karbird/` and `apps/web/public/brand/`.

## Evidence on Hand

- Working end-to-end demo stand (`just dev` + `just demo`) with fault injection that visibly turns map nodes amber/red.
- Feature and status truth: `README.md`, `TASKS.md`, `apps/web/src/app/(public)/features/page.tsx`, ADRs in `docs/adr/`.
- Absent and must not be fabricated: customers, testimonials, case studies, published benchmarks or load numbers, pricing, certifications, or a verified live production deployment.

## Product Principles

1. **Truth over polish.** Show observed vs inferred, completeness and gaps explicitly; never present guessed state as fact.
2. **Time is a first-class axis.** Every view should answer "what is happening" and "what was happening then" with the same interaction.
3. **Change only through control.** Any action that touches the observed system is typed, approved, verified and audited; the UI never offers a shortcut around that.
4. **Safe by default.** Untrusted telemetry, tenant isolation and redaction are product features, not settings.
5. **One workspace for the whole loop.** Observe, investigate and act without switching tools — for humans and for their AI assistants alike.
