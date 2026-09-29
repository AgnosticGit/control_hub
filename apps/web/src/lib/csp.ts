// Content-Security-Policy for the web app.
//
// - `/app/*` (renders telemetry, holds the session) gets a strict per-request nonce policy from
//   `proxy.ts`: no inline script without the nonce, no eval.
// - Everything else (static public pages, assets, the engine Worker script) gets the baseline
//   below. Static pages cannot carry a per-request nonce, so it allows inline scripts, but still
//   only from this origin, with no plugins, framing or foreign form targets.
// - `'wasm-unsafe-eval'` allows compiling WebAssembly only (the engine), not JS eval. The Pixi
//   renderer imports `pixi.js/unsafe-eval`, which removes its need for `new Function`.
// - `connect-src 'self'` covers same-origin fetch and WebSocket (`/api/v1/*` is proxied).

const dev = process.env.NODE_ENV !== "production";

function policy(scriptSrc: string[]): string {
  return [
    "default-src 'self'",
    `script-src ${[...scriptSrc, "'wasm-unsafe-eval'", ...(dev ? ["'unsafe-eval'"] : [])].join(" ")}`,
    // React style attributes need 'unsafe-inline' (a nonce here would disable it).
    "style-src 'self' 'unsafe-inline'",
    "img-src 'self' data: blob:",
    "font-src 'self' data:",
    "connect-src 'self'",
    "worker-src 'self' blob:",
    "object-src 'none'",
    "base-uri 'self'",
    "form-action 'self'",
    "frame-ancestors 'none'",
  ].join("; ");
}

export const baselineCsp = policy(["'self'", "'unsafe-inline'"]);

export function strictCsp(nonce: string): string {
  return policy([`'nonce-${nonce}'`, "'strict-dynamic'", "'self'"]);
}
