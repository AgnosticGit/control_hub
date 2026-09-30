# ControlHub developer commands. Run `just` to list them.
# Every recipe is a thin wrapper over a script, so it works the same on
# Windows (PowerShell) and Unix shells.

set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

default:
    @just --list

# Check required tools, Docker and local ports (never installs anything)
doctor:
    node scripts/doctor.mjs

# Install JS deps, generate local secrets, start infra, migrate and seed
bootstrap:
    pnpm install --frozen-lockfile
    node scripts/bootstrap.mjs

# Build and run ingest + processor + API + executor + web (Ctrl+C stops all).
# `just dev ngrok` also exposes the web app at CONTROLHUB_PUBLIC_URL through ngrok.
dev *ARGS:
    node scripts/dev.mjs {{ARGS}}

# Same as dev without the web app
dev-backend:
    node scripts/dev.mjs --no-web

# Stop leftovers of `just dev` / `just demo` (e.g. after closing their terminal); keeps containers
stop:
    node scripts/stop.mjs

# Start the demo system (7 services, Postgres, a payment provider stand-in, load generator)
demo:
    node scripts/demo.mjs

# Demo-only fault control, e.g. `just demo-fault orders --latency 800 --error-rate 0.3`
demo-fault *ARGS:
    node examples/demo-system/src/fault.mjs {{ARGS}}

# Build the Rust engine to WASM and publish the content-hashed artifact to the web app
wasm:
    node scripts/build-wasm.mjs

# Regenerate TS types (ts-rs), OpenAPI spec and the TS API client
codegen:
    node scripts/codegen.mjs

# Release build of services, WASM engine and web app
build: wasm
    cargo build --release -p controlhub-ingest -p controlhub-processor -p controlhub-api -p controlhub-executor -p controlhub-cli
    pnpm --filter @controlhub/web build

# Formatting, lints and type checks (no running services needed)
check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo check -p controlhub-engine-core -p controlhub-web-engine --target wasm32-unknown-unknown
    pnpm -r --if-present typecheck
    pnpm --filter @controlhub/web lint

# Unit and property tests (Rust + JS), no running services needed
test:
    cargo test --workspace
    pnpm --filter @controlhub/sdk-node test
    pnpm --filter @controlhub/web test
    node --test "examples/demo-system/test/*.test.mjs" "scripts/test/*.test.mjs"

# Integration tests against the running stack (needs: just dev + just demo).
# Test accounts exist only for the duration of the run.
test-integration:
    cargo build -q -p controlhub-cli
    node scripts/test-with-fixtures.mjs node --test "tests/integration/*.test.mjs"

# Disruptive: stops Kafka, expects 503 from ingest, restarts it and checks recovery
test-broker-outage:
    node scripts/test-with-fixtures.mjs node tests/integration/broker-outage.test.mjs --run

# The controlhub CLI, e.g. `just cli discover --path ../my-app`
# (`exit $LASTEXITCODE` keeps the CLI's exit code under PowerShell; in sh it expands to plain `exit`)
cli *ARGS:
    cargo run -q -p controlhub-cli -- {{ARGS}}; exit $LASTEXITCODE

# Browser tests against the running stack (needs: just dev + just demo).
# Test accounts exist only for the duration of the run.
test-e2e:
    node scripts/test-with-fixtures.mjs pnpm --filter @controlhub/e2e test

# Reproducible engine benchmark on synthetic data
bench:
    cargo run --release -p controlhub-engine-core --example bench

# Stop infrastructure containers; data volumes are kept
down:
    node scripts/down.mjs

# DESTRUCTIVE: delete all local data volumes (asks for confirmation), then bootstrap a fresh stand
# incl. the owner account from .controlhub/owner.env. `just reset --no-bootstrap` only deletes.
reset *ARGS:
    node scripts/reset.mjs {{ARGS}}
