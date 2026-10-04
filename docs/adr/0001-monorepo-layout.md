# ADR-0001: Monorepo layout

## Context
karbird has Rust services, a Rust engine that ships as both a native build and WASM, a Next.js
app, TS packages and a Node SDK. Contracts between them (engine types and OpenAPI) are generated
from Rust.

## Decision
- One repository with a Cargo workspace (`crates/*`) and a pnpm workspace (`apps/*`,
  `packages/*`, `examples/demo-system`, `tests/e2e`).
- Rust toolchain pinned in `rust-toolchain.toml`. Node version via `engines`; pnpm version via
  `packageManager`.
- Every developer command is a Node script in `scripts/`, so it behaves the same under PowerShell
  and POSIX shells. The `justfile` only wraps those scripts.
- Generated contracts (`schemas/generated`, `packages/*/src/generated`) are committed, and CI
  regenerates them and fails on drift. WASM build output is not committed.

## Consequences
- A change to a Rust type and to its TS consumers can land in one reviewable commit.
- Building the web app needs the Rust toolchain plus `wasm-bindgen-cli`.
