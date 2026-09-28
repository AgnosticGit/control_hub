# ControlHub

ControlHub is a platform for observability, history exploration and (later) controlled
management of distributed systems. It takes in standard OpenTelemetry data, keeps its history
in ClickHouse, and shows the system as a live map with a time machine. The map runs on a Rust
engine compiled to WebAssembly and is rendered with WebGL.

> **Status:** early. Stages 0–1 and most of stage 2 work end to end on a local demo stand.
> The CLI, the actions pipeline, MCP, WebSocket realtime and OTLP metrics are not implemented yet.
> See [STATUS.md](STATUS.md) for what was actually verified.

```
SDK / OTel Collector ──OTLP──▶ ingest (Rust) ──▶ Kafka ──▶ processor (Rust) ──▶ ClickHouse
                                  │ ACK only after broker ACK                    │
                                  ▼                                              ▼
                             PostgreSQL (tenants, tokens, sessions, offsets, audit)  API (Rust, /api/v1)
                                                                                 │
                              Browser: Next.js ◀── Worker ◀── WASM engine ◀──────┘
                                        └─ PixiJS map, inspector, logs/traces/events, timeline
```

## Requirements

- Rust (installed via rustup; the version is pinned in `rust-toolchain.toml`), plus `wasm-bindgen-cli 0.2.129`
- Node.js 22.12 or newer, and pnpm 10 (`corepack enable`)
- Docker with Compose v2
- [`just`](https://github.com/casey/just) (optional: every recipe is just `node scripts/<name>.mjs`)

Run `just doctor` to check what is missing. The scripts never install system packages.

## Quick start (local demo)

```
just bootstrap      # pnpm install, generate secrets into .controlhub/, start PG/ClickHouse/Kafka, migrate, seed
just wasm           # build the WASM engine into apps/web/public/wasm
just dev            # ingest :4318/:4317, processor, API :8080, web :3000
just demo           # in a second terminal: gateway :7001, orders :7002, demo Postgres, load generator
```

Open http://127.0.0.1:3000/app and sign in with the credentials from `.controlhub/local-credentials.json`.
They are generated fresh on every bootstrap; there is no default admin password.

To inject a fault, which only works on the demo stand and only from localhost:

```
just demo-fault orders --latency 150 --error-rate 0.6
just demo-fault orders --clear
```

Within a few seconds the `orders` node on the map turns amber or red. Error traces and correlated ERROR logs
show up in the bottom panel, and you can scrub the timeline back to before the fault.

## Checks

```
just check              # rustfmt, clippy -D warnings, wasm32 check, TS typecheck, eslint
just test               # Rust unit + property tests, SDK tests
just test-integration   # needs `just dev` + `just demo`
just test-e2e           # Playwright; needs `just dev` + `just demo` (or `pnpm --filter @controlhub/web start`)
just bench              # engine benchmark on synthetic data
```

## Documentation

- [STATUS.md](STATUS.md) — verified progress, check results, limitations
- [TASKS.md](TASKS.md) — next work, by stage
- [AGENTS.md](AGENTS.md) — repository map and rules for contributors and coding agents
- [docs/architecture/data-flow.md](docs/architecture/data-flow.md)
- [docs/security/README.md](docs/security/README.md)
- [docs/operations/local-dev.md](docs/operations/local-dev.md)
- [docs/integration/AI-INTEGRATION.md](docs/integration/AI-INTEGRATION.md)
- [docs/adr/](docs/adr/) — architecture decision records
