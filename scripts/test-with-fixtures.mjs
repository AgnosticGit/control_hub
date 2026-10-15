#!/usr/bin/env node
// Runs a test command with test accounts and a clean reserved telemetry scope, then removes
// those fixtures (also when the tests fail), so local runs stay repeatable.
// Usage: node scripts/test-with-fixtures.mjs <command> [args...]
import { exitCode, loadEnvFile, run, withFixtures } from "./lib.mjs";

const [cmd, ...args] = process.argv.slice(2);
if (!cmd) {
  console.error("usage: node scripts/test-with-fixtures.mjs <command> [args...]");
  process.exit(2);
}

const infra = loadEnvFile(".karbird/infra.env");
if (!infra.CH_PG_ADMIN_PASSWORD) {
  console.error("missing .karbird/infra.env — run `just bootstrap` first");
  process.exit(2);
}
const fixtures = (extra) =>
  run("cargo", ["run", "-q", "-p", "karbird-api", "--", "test-fixtures", "--out-dir", ".karbird", ...extra], {
    env: { ...process.env, ...infra },
  });

const setDemoIncidentsEnabled = async (enabled) => {
  try {
    const response = await fetch("http://127.0.0.1:7090/incident-cycle", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ enabled }),
      signal: AbortSignal.timeout(2000),
    });
    if (!response.ok) throw new Error(`demo incident-cycle control returned ${response.status}`);
  } catch (error) {
    if (error instanceof TypeError && error.cause?.code === "ECONNREFUSED") return;
    throw error;
  }
};

process.exitCode = await withFixtures({
  setup: () => fixtures([]),
  body: async () => {
    await setDemoIncidentsEnabled(false);
    try {
      return await exitCode(cmd, args);
    } finally {
      await setDemoIncidentsEnabled(true);
    }
  },
  teardown: () => fixtures(["--remove"]),
});
