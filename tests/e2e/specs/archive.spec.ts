import { spawnSync } from "node:child_process";
import { join } from "node:path";
import { expect, test } from "./isolated-test";
import { ingestLog, login, randomHex, root } from "./helpers";

// The log archive as a user sees it (ADR-0015): an hour older than the API's hot window (13 days),
// archived by `karbird-archiver once`, opened through Range on the timeline. Writes only into
// the isolation tenant's `isolation-test` environment.

const WORKSPACE = "/app/isolation/isolation-test";
const LOGS_API = "/api/v1/projects/isolation/environments/isolation-test/logs";
const HOUR = 3_600_000_000_000n;
const DAY = 24n * HOUR;
const bin = join(root, "target", "debug", process.platform === "win32" ? "karbird-archiver.exe" : "karbird-archiver");

function localInput(ns: bigint) {
  const d = new Date(Number(ns / 1_000_000n));
  return new Date(d.getTime() - d.getTimezoneOffset() * 60_000).toISOString().slice(0, 16);
}

test.beforeAll(() => {
  const build = spawnSync("cargo", ["build", "-q", "-p", "karbird-archiver"], { cwd: root, stdio: "inherit" });
  expect(build.status, "cargo build -p karbird-archiver").toBe(0);
});

test("logs older than the hot window open from the archive, labelled, filterable and switchable", async ({ page }) => {
  test.setTimeout(180_000);
  const run = randomHex(4);
  const service = `archive-ui-${run}`;
  const start = ((BigInt(Date.now()) * 1_000_000n - 13n * DAY) / HOUR) * HOUR - 2n * HOUR;
  const end = start + HOUR;
  const at = end - 120_000_000_000n;
  const bodies = [`archived order 17 shipped run ${run}`, `archived order 42 shipped run ${run}`, `archived cache warmed run ${run}`];
  for (const [i, body] of bodies.entries()) {
    await ingestLog({ service, body, severity: 9, timeNs: at + BigInt(i) * 1_000_000_000n });
  }

  await login(page, "isolation@karbird.local", WORKSPACE);
  await expect
    .poll(
      async () => {
        const r = await page.request.get(`${LOGS_API}?from=${start}&to=${end}&q=${run}&source=hot`);
        return r.ok() ? ((await r.json()) as { logs: unknown[] }).logs.length : -1;
      },
      { timeout: 60_000, message: "the old logs reach ClickHouse" },
    )
    .toBe(bodies.length);

  const archived = spawnSync(
    bin,
    ["once", "--tenant", "isolation-tenant", "--project", "isolation", "--environment", "isolation-test", "--from-ns", String(start), "--to-ns", String(end), "--min-age-secs", "0"],
    { cwd: root, encoding: "utf8", timeout: 120_000 },
  );
  expect(archived.status, archived.stderr).toBe(0);

  const timeline = page.getByLabel("Timeline");
  await timeline.getByRole("button", { name: "Range", exact: true }).click();
  await expect(timeline.getByText(/Logs older than about two weeks come from the archive/)).toBeVisible();
  await timeline.getByLabel("From", { exact: true }).fill(localInput(start));
  await timeline.getByLabel("To", { exact: true }).fill(localInput(end));
  await timeline.getByRole("button", { name: "Load range" }).click();
  await expect(timeline.getByText("historical", { exact: true })).toBeVisible();
  await expect(timeline.getByText("Logs from archive · no traces")).toBeVisible({ timeout: 30_000 });

  const panel = page.getByRole("region", { name: "Signals" });
  await panel.getByLabel("Search log text").fill(`run ${run}`);
  const records = panel.getByLabel("Log records");
  for (const body of bodies) await expect(records).toContainText(body, { timeout: 30_000 });
  await expect(panel.getByText("From archive", { exact: true })).toBeVisible();
  await expect(panel.getByRole("group", { name: "Log source" }).getByRole("button", { name: "Auto" })).toHaveAttribute("aria-pressed", "true");

  // "like": only lines of the same kind, numbers aside.
  await records.getByRole("listitem").filter({ hasText: bodies[0]! }).getByRole("button", { name: "Show lines like this" }).click();
  await expect(panel.getByText("Lines like")).toBeVisible();
  await expect(records).toContainText(bodies[0]!);
  await expect(records).toContainText(bodies[1]!);
  await expect(records).not.toContainText(bodies[2]!);
  await panel.getByRole("button", { name: "Show all lines" }).click();
  await expect(records).toContainText(bodies[2]!);

  // The same hour is still in ClickHouse: the hot store shows it without the archive label.
  const source = panel.getByRole("group", { name: "Log source" });
  await source.getByRole("button", { name: "Hot" }).click();
  await expect(source.getByRole("button", { name: "Hot" })).toHaveAttribute("aria-pressed", "true");
  await expect(records).toContainText(bodies[2]!);
  await expect(panel.getByText("From archive", { exact: true })).toHaveCount(0);
  await source.getByRole("button", { name: "Archive" }).click();
  await expect(panel.getByText("From archive", { exact: true })).toBeVisible();
  for (const body of bodies) await expect(records).toContainText(body);
});
