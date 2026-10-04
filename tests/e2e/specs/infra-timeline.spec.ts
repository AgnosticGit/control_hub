import { expect, test, type Page } from "@playwright/test";
import { collectErrors, ingestMetrics, login, randomHex, type MetricInput } from "./helpers";

const WORKSPACE = "/app/isolation/isolation-test";
const MIB = 1024 * 1024;
const SEC = 1_000_000_000n;

function clock(ns: bigint): string {
  const d = new Date(Number(ns / 1_000_000n));
  return [d.getHours(), d.getMinutes(), d.getSeconds()].map((v) => String(v).padStart(2, "0")).join(":");
}

async function cursorText(page: Page): Promise<string> {
  return (await page.getByLabel("Timeline").getByRole("slider", { name: "Selected time" }).getAttribute("aria-valuetext")) ?? "";
}

test("machine track on the timeline: CPU and memory per machine and a container restart to jump to", async ({ page }) => {
  const run = randomHex(4);
  const host = `e2e-tl-vps-${run}`;
  const web = `web-${run}`;
  const now = BigInt(Date.now()) * 1_000_000n;
  // Points every 20 s over the last 10 minutes; the container's CPU counter starts over at point 15.
  const from = now - 600n * SEC;
  const at = (i: number) => from + BigInt(i) * 20n * SEC;
  const all = Array.from({ length: 30 }, (_, i) => i);
  const restartAt = at(15);
  const sum = (name: string, unit: string, monotonic: boolean, points: Array<{ attributes?: Record<string, string>; value: number; timeNs: bigint }>): MetricInput => ({
    name,
    unit,
    kind: "sum",
    monotonic,
    points: points.map((p) => ({ ...p, startNs: from - SEC })),
  });
  await ingestMetrics([
    {
      attributes: { "host.name": host, "host.id": host },
      metrics: [
        sum("system.cpu.time", "s", true, [
          ...all.map((i) => ({ attributes: { cpu: "cpu0", state: "user" }, value: 1000 + i * 10, timeNs: at(i) })),
          ...all.map((i) => ({ attributes: { cpu: "cpu0", state: "idle" }, value: 1000 + i * 10, timeNs: at(i) })),
        ]),
        sum("system.memory.usage", "By", false, [
          ...all.map((i) => ({ attributes: { state: "used" }, value: 512 * MIB, timeNs: at(i) })),
          ...all.map((i) => ({ attributes: { state: "free" }, value: 512 * MIB, timeNs: at(i) })),
        ]),
        { name: "system.memory.limit", unit: "By", kind: "gauge", points: all.map((i) => ({ value: 1024 * MIB, timeNs: at(i) })) },
      ],
    },
    {
      attributes: { "host.name": host, "host.id": host, "container.id": randomHex(32), "container.name": `/${web}` },
      metrics: [
        sum("container.cpu.usage.total", "ns", true, all.map((i) => ({ value: i < 15 ? 1e12 + i * 1e9 : (i - 15) * 1e9, timeNs: at(i) }))),
        { name: "container.memory.usage.total", unit: "By", kind: "gauge", points: all.map((i) => ({ value: 64 * MIB, timeNs: at(i) })) },
      ],
    },
  ]);

  const errors = collectErrors(page);
  await login(page, "isolation@karbird.local", WORKSPACE);
  await expect(page.getByText("Starting engine…")).toBeHidden();
  const timeline = page.getByLabel("Timeline");
  const tracks = timeline.getByRole("list", { name: "Tracks" });
  const restart = timeline.getByRole("button", { name: `Container restarted: ${web} on ${host} at ${clock(restartAt)}` });

  // Metrics reach the workspace through the pipeline; reload until the machine and its event are there.
  await expect
    .poll(
      async () => {
        if ((await restart.count()) > 0) return true;
        await page.reload();
        await expect(page.getByText("Starting engine…")).toBeHidden();
        return false;
      },
      { timeout: 60_000, intervals: [3_000] },
    )
    .toBe(true);

  await expect(tracks.getByText("Machines", { exact: true })).toBeVisible();
  const row = tracks.getByRole("listitem").filter({ hasText: host });
  await expect(row).toBeVisible();
  await expect(restart).toHaveAttribute("title", `${web} restarted on ${host}`);

  // A click on the event moves the cursor there, and the machine row reads that moment.
  await restart.click();
  await expect.poll(() => cursorText(page)).toContain(clock(restartAt));
  await expect(timeline.getByRole("button", { name: "Pause live" })).toHaveCount(0);
  await expect(row).toContainText("50% · 50%");
  await page.screenshot({ path: "../../test-results/e2e/infra-timeline.png" });
  expect(errors).toEqual([]);
});
