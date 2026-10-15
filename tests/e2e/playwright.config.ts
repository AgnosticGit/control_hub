import { defineConfig, devices } from "@playwright/test";

// Expects a running stack: `just dev` (or `pnpm --filter @karbird/web start`
// + backend) and `just demo` for telemetry. Base URL is overridable.
const baseURL = process.env.KARBIRD_WEB_URL ?? "http://127.0.0.1:3000";

export default defineConfig({
  testDir: "./specs",
  timeout: 60_000,
  expect: { timeout: 20_000 },
  fullyParallel: false,
  // Specs share one demo system (fault injection, action plans): never run files concurrently.
  workers: 1,
  retries: 0,
  reporter: [["list"], ["./isolation-reporter.mjs"]],
  outputDir: "../../test-results/e2e",
  use: {
    baseURL,
    // ngrok's free plan puts a one-time warning page in front of browsers; skip it when testing through the tunnel.
    extraHTTPHeaders: /\.ngrok-free\.(dev|app)$/.test(new URL(baseURL).hostname) ? { "ngrok-skip-browser-warning": "1" } : undefined,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1600, height: 1000 } } }],
});
