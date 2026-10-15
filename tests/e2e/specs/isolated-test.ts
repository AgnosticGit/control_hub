import { execFileSync } from "node:child_process";
import { test as base } from "@playwright/test";
import { createSpecIsolation } from "../spec-isolation.mjs";
import { loadEnvFile, root } from "../../../scripts/lib.mjs";

export * from "@playwright/test";

const infra = loadEnvFile(".karbird/infra.env");
const env = { ...process.env, ...infra };
const prepare = createSpecIsolation({
  drain: () => execFileSync(process.execPath, ["tests/e2e/drain-telemetry.mjs"], { cwd: root, env, stdio: "inherit" }),
  clear: () => execFileSync("cargo", ["run", "-q", "-p", "karbird-api", "--", "test-fixtures", "--clear-telemetry"], {
    cwd: root, env, stdio: "inherit",
  }),
});

export const test = base.extend<{ specIsolation: void }>({
  specIsolation: [async ({}, use, testInfo) => {
    prepare(testInfo.file);
    await use();
  }, { auto: true, timeout: 120_000 }],
});
