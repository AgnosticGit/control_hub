#!/usr/bin/env node
// Runs a test command with the test accounts and scopes in place, then deletes the accounts
// again (also when the tests fail), so the local database keeps only the owner account.
// Usage: node scripts/test-with-fixtures.mjs <command> [args...]
import { exitCode, loadEnvFile, run, withFixtures } from "./lib.mjs";

const [cmd, ...args] = process.argv.slice(2);
if (!cmd) {
  console.error("usage: node scripts/test-with-fixtures.mjs <command> [args...]");
  process.exit(2);
}

const infra = loadEnvFile(".controlhub/infra.env");
if (!infra.CH_PG_ADMIN_PASSWORD) {
  console.error("missing .controlhub/infra.env — run `just bootstrap` first");
  process.exit(2);
}
const fixtures = (extra) =>
  run("cargo", ["run", "-q", "-p", "controlhub-api", "--", "test-fixtures", "--out-dir", ".controlhub", ...extra], {
    env: { ...process.env, ...infra },
  });

process.exitCode = await withFixtures({
  setup: () => fixtures([]),
  body: () => exitCode(cmd, args),
  teardown: () => fixtures(["--remove"]),
});
