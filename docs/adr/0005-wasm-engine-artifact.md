# ADR-0005: WASM engine in a Worker, content-hashed artifact

## Context
The timeline and topology engine must be deterministic and fast, and must not block the UI
thread. The same code should also run natively for tests and benchmarks.

## Decision
- `controlhub-engine-core` is pure Rust with no I/O. `controlhub-web-engine` wraps it with
  wasm-bindgen (`--target web`).
- It runs inside a dedicated Web Worker. Frames are flat typed arrays that are *transferred*
  (not copied) to the UI thread.
- `scripts/build-wasm.mjs` builds with the `wasm-release` profile and copies the module to
  `apps/web/public/wasm/controlhub_engine.<sha256-16>.wasm`. It also writes
  `src/generated/wasm-manifest.ts` (URL, SHA-256, size), and `/wasm/*` is served as immutable.
- `ENGINE_ABI_VERSION` is compiled into both Rust and TS, and the Worker refuses to start if they
  differ.
- wasm-bindgen CLI and crate versions must match exactly (0.2.129). The build script checks this.

## Consequences
- A deploy never pairs stale cached WASM with new JS.
- The web build depends on `just wasm` having been run.
