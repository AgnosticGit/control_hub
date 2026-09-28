import { defineConfig, devices } from "@playwright/test";

// Expects a running stack: `just dev` (or `pnpm --filter @controlhub/web start`
// + backend) and `just demo` for telemetry. Base URL is overridable.
export default defineConfig({
  testDir: "./specs",
  timeout: 60_000,
  expect: { timeout: 20_000 },
  fullyParallel: false,
  retries: 0,
  reporter: [["list"]],
  outputDir: "../../test-results/e2e",
  use: {
    baseURL: process.env.CONTROLHUB_WEB_URL ?? "http://127.0.0.1:3000",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1600, height: 1000 } } }],
});
