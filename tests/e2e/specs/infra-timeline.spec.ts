import { expect, test, type Page, type Request } from "@playwright/test";
import { collectErrors, ingestMetrics, ingestSpans, login, randomHex, type MetricInput } from "./helpers";

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
  await ingestSpans(`machine-timeline-${run}`, [{
    traceId: randomHex(16),
    spanId: randomHex(8),
    name: "machine timeline anchor",
    kind: 2,
    startNs: now - 30n * SEC,
    endNs: now - 20n * SEC,
    attributes: { "host.name": host },
  }]);

  const errors = collectErrors(page);
  await login(page, "isolation@karbird.local", WORKSPACE);
  await expect(page.getByText("Starting engine…")).toBeHidden();
  const timeline = page.getByLabel("Timeline");
  const tracks = timeline.getByRole("list", { name: "Tracks" });
  const zoomToMachineDetail = async () => {
    for (let i = 0; i < 2; i++) await page.getByLabel("Timeline").getByRole("button", { name: "Zoom in time" }).click();
  };
  await zoomToMachineDetail();
  const restart = timeline.getByRole("button", { name: `Container restarted: ${web} on ${host} at ${clock(restartAt)}` });

  // The ingest response is a Kafka ACK; wait for the processor's ClickHouse result
  // before loading the timeline once, instead of repeatedly interrupting page loads.
  const scope = "/api/v1/projects/isolation/environments/isolation-test";
  await expect.poll(async () => {
    const response = await page.request.get(
      `${scope}/infrastructure/timeline?from=${from}&to=${now + 60n * SEC}&buckets=200`,
    );
    if (!response.ok()) return false;
    const result = await response.json() as { events: Array<{ kind: string; host_name: string; container_name: string }> };
    return result.events.some((event) => event.kind === "restarted" && event.host_name === host && event.container_name === web);
  }, { timeout: 60_000 }).toBe(true);
  await page.reload();
  await expect(page.getByRole("application", { name: /Service map/ })).toBeVisible();
  await zoomToMachineDetail();
  await expect(restart).toBeVisible();

  await expect(tracks.getByText("Machines", { exact: true })).toBeVisible();
  const row = tracks.getByRole("listitem").filter({ hasText: host });
  await expect(row).toBeVisible();
  await expect(restart).toHaveAttribute("title", `${web} restarted on ${host}`);

  // A click on the event moves the cursor there, and the machine row reads that moment.
  await restart.click();
  await expect.poll(() => cursorText(page)).toContain(clock(restartAt));
  await expect(timeline.getByRole("button", { name: "Pause live" })).toHaveCount(0);
  await expect(row).toContainText("50% · 50%");

  // A viewport change redraws the canvas, but must not refetch the same infrastructure window.
  const ruler = timeline.locator("canvas").first();
  const oldWidth = await ruler.getAttribute("width");
  const resizeRequests: string[] = [];
  const onRequest = (request: Request) => {
    if (request.url().includes("/infrastructure/timeline")) resizeRequests.push(request.url());
  };
  page.on("request", onRequest);
  try {
    await page.setViewportSize({ width: 1200, height: 1000 });
    await expect.poll(() => ruler.getAttribute("width")).not.toBe(oldWidth);
    await page.evaluate(
      () => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))),
    );
    expect(resizeRequests).toEqual([]);
  } finally {
    page.off("request", onRequest);
  }
  await page.screenshot({ path: "../../test-results/e2e/infra-timeline.png" });
  expect(errors).toEqual([]);
});
