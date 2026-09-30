# ControlHub

ControlHub is a platform for observability, history exploration and (later) controlled
management of distributed systems. It takes in standard OpenTelemetry data, keeps its history
in ClickHouse, and shows the system as a live map with a time machine. The map runs on a Rust
engine compiled to WebAssembly and is rendered with WebGL.

> **Status:** not production-ready. On a local demo stand the following work end to end and are
> tested: telemetry flow, map and time machine, WebSocket live updates, token management, the
> `controlhub` CLI, controlled actions (demo adapter only) and a read-only MCP server. OTLP metrics,
> archiving, production deployment and real (non-demo) action adapters do not exist.

```
SDK / OTel Collector ──OTLP──▶ ingest (Rust) ──▶ Kafka ──▶ processor (Rust) ──▶ ClickHouse
                                  │ ACK only after broker ACK                    │
                                  ▼                                              ▼
                             PostgreSQL (tenants, tokens, sessions, offsets, audit)  API (Rust, /api/v1)
                                                                                 │
                              Browser: Next.js ◀── Worker ◀── WASM engine ◀──────┘
                                        └─ PixiJS map, inspector, logs/traces/events, timeline

Actions: web → API (plan, policy, four-eyes approval) → PostgreSQL ← executor (separate process,
         demo adapter, verification from ClickHouse, audit)
CLI:     controlhub discover / plan / apply / verify / context / mcp (read-only API token)
```

## Requirements

- Rust (installed via rustup; the version is pinned in `rust-toolchain.toml`), plus `wasm-bindgen-cli 0.2.129`
- Node.js 22.12 or newer, and pnpm 10 (`corepack enable`)
- Docker with Compose v2
- [`just`](https://github.com/casey/just) (optional: every recipe is just `node scripts/<name>.mjs`)

Run `just doctor` to check what is missing. The scripts never install system packages.

## Quick start (local demo)

First create `.controlhub/owner.env` (gitignored) with your own account:

```
CONTROLHUB_OWNER_EMAIL=you@example.com
CONTROLHUB_OWNER_PASSWORD=a long password
```

```
just bootstrap      # pnpm install, generate secrets into .controlhub/, start PG/ClickHouse/Kafka, migrate, seed
just wasm           # build the WASM engine into apps/web/public/wasm
just dev            # ingest :4318/:4317, processor, API :8080, web :3000
just demo           # in a second terminal: gateway :7001, orders :7002, demo Postgres, load generator
```

Open http://127.0.0.1:3000/app and sign in with that owner account (owner of the demo project). It is
the only account a bootstrap creates; there is no default admin password. Test accounts are created
by `just test-integration` / `just test-e2e` for the duration of the run and deleted afterwards
(`controlhub-api test-fixtures [--remove]`, emails in `@controlhub.local` / `@example.test`).

To inject a fault, which only works on the demo stand and only from localhost:

```
just demo-fault orders --latency 150 --error-rate 0.6
just demo-fault orders --clear
```

Within a few seconds the `orders` node on the map turns amber or red. Error traces and correlated ERROR logs
show up in the bottom panel, and you can scrub the timeline back to before the fault.

The same faults can go through the controlled-actions flow instead: open **Actions** and propose
"Inject faults" on `orders`, then approve it as a second user of the project (the e2e suite does this
with its temporary operator and admin accounts). The executor (started by `just dev`)
applies the change, verifies it from telemetry about 25 s later, and the plan detail shows the
policy checks, the result, the verification and the audit trail.

Integrating your own Node service and giving a coding assistant read-only context:
[docs/integration/CLI.md](docs/integration/CLI.md).

## Checks

```
just check              # rustfmt, clippy -D warnings, wasm32 check, TS typecheck, eslint
just test               # Rust unit + property tests, SDK tests
just test-integration   # needs `just dev` + `just demo`
just test-broker-outage # disruptive: stops Kafka, expects 503, checks recovery
just test-e2e           # Playwright; needs `just dev` + `just demo` (or `pnpm --filter @controlhub/web start`)
just bench              # engine benchmark on synthetic data
```

## Documentation

- [TASKS.md](TASKS.md) — next work, by stage
- [AGENTS.md](AGENTS.md) — repository map and rules for contributors and coding agents
- [docs/architecture/data-flow.md](docs/architecture/data-flow.md)
- [docs/security/README.md](docs/security/README.md)
- [docs/operations/local-dev.md](docs/operations/local-dev.md)
- [docs/integration/AI-INTEGRATION.md](docs/integration/AI-INTEGRATION.md)
- [docs/adr/](docs/adr/) — architecture decision records
