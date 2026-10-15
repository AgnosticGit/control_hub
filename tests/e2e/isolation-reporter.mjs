import { execFileSync } from "node:child_process";
import { loadEnvFile, root } from "../../scripts/lib.mjs";

const infra = loadEnvFile(".karbird/infra.env");

/** Keep every browser test independent of telemetry sent by earlier tests. */
export default class IsolationReporter {
  lastFile = null;

  onTestBegin(test) {
    // Tests in a spec use unique synthetic names; reset between specs, not between every
    // browser case. This keeps ClickHouse from running hundreds of synchronous mutations
    // during a full E2E run while still isolating files from one another.
    if (this.lastFile === test.location.file) return;
    if (this.lastFile !== null) {
      execFileSync(process.execPath, ["tests/e2e/drain-telemetry.mjs"], {
        cwd: root,
        env: { ...process.env, ...infra },
        stdio: "inherit",
      });
    }
    this.lastFile = test.location.file;
    execFileSync("cargo", ["run", "-q", "-p", "karbird-api", "--", "test-fixtures", "--clear-telemetry"], {
      cwd: root,
      env: { ...process.env, ...infra },
      stdio: "inherit",
    });
  }
}
