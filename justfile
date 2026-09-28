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

# Build and run ingest + processor + API + web (Ctrl+C stops all)
dev:
    node scripts/dev.mjs

# Same as dev without the web app
dev-backend:
    node scripts/dev.mjs --no-web

# Start the demo system (gateway, orders, Postgres, load generator)
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
    cargo build --release -p controlhub-ingest -p controlhub-processor -p controlhub-api
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

# Integration tests against the running stack (needs: just dev + just demo)
test-integration:
    node --test "tests/integration/*.test.mjs"

# Browser tests against the running stack (needs: just dev + just demo)
test-e2e:
    pnpm --filter @controlhub/e2e test

# Reproducible engine benchmark on synthetic data
bench:
    cargo run --release -p controlhub-engine-core --example bench

# Stop infrastructure containers; data volumes are kept
down:
    node scripts/down.mjs

# DESTRUCTIVE: stop infrastructure and delete all local data volumes (asks for confirmation)
reset:
    node scripts/reset.mjs
