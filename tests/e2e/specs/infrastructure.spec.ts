import { expect, test } from "@playwright/test";
import { collectErrors, ingestMetrics, ingestSpans, login, randomHex, type MetricInput, type SpanInput } from "./helpers";

const WORKSPACE = "/app/isolation/isolation-test";
const MIB = 1024 * 1024;
const SEC = 1_000_000_000n;

function serverSpans(n: number, fromNs: bigint, ms: number): SpanInput[] {
  return Array.from({ length: n }, (_, i) => {
    const start = fromNs + BigInt(i) * SEC;
    return { traceId: randomHex(16), spanId: randomHex(8), name: "GET /x", kind: 2, startNs: start, endNs: start + BigInt(ms) * 1_000_000n };
  });
}

test("infrastructure layer: machines, noisy neighbour, service → instance → container → machine, signals of the moment", async ({ page }) => {
  const run = randomHex(4);
  const host = `e2e-vps-${run}`;
  const orders = `orders-${run}`;
  const inventory = `inventory-${run}`;
  const [ordersCtr, inventoryCtr] = [randomHex(32), randomHex(32)];
  const now = BigInt(Date.now()) * 1_000_000n;
  const t1 = now - 100n * SEC;
  const t2 = now - 20n * SEC;

  const sum = (name: string, unit: string, rows: Array<[Record<string, string>, number, number]>, monotonic = true): MetricInput => ({
    name,
    unit,
    kind: "sum",
    monotonic,
    points: rows.flatMap(([attributes, a, b]) => [
      { attributes, value: a, startNs: t1 - SEC, timeNs: t1 },
      { attributes, value: b, startNs: t1 - SEC, timeNs: t2 },
    ]),
  });
  const gauge = (name: string, unit: string, value: number): MetricInput => ({ name, unit, kind: "gauge", points: [{ value, timeNs: t2 }] });
  const container = (id: string, name: string, cores: number) => ({
    attributes: { "host.name": host, "host.id": host, "container.id": id, "container.name": name },
    metrics: [sum("container.cpu.usage.total", "ns", [[{}, 1e12, 1e12 + cores * 80e9]]), gauge("container.memory.usage.total", "By", 200 * MIB)],
  });
  // One core 95 % busy over 80 s; inventory takes 0.95 of it.
  await ingestMetrics([
    {
      attributes: { "host.name": host, "host.id": host },
      metrics: [
        sum("system.cpu.time", "s", [
          [{ cpu: "cpu0", state: "user" }, 1000, 1076],
          [{ cpu: "cpu0", state: "idle" }, 1000, 1004],
        ]),
        gauge("system.cpu.logical.count", "{cpu}", 1),
        sum("system.memory.usage", "By", [[{ state: "used" }, 870 * MIB, 870 * MIB], [{ state: "free" }, 154 * MIB, 154 * MIB]], false),
        gauge("system.memory.limit", "By", 1024 * MIB),
        sum("system.filesystem.usage", "By", [[{ mountpoint: "/", state: "used" }, 9e9, 9e9], [{ mountpoint: "/", state: "free" }, 16e9, 16e9]], false),
      ],
    },
    container(ordersCtr, orders, 0.02),
    container(inventoryCtr, inventory, 0.95),
  ]);
  const ordersRes = { "service.instance.id": `${orders}-1`, "container.id": ordersCtr };
  await ingestSpans(orders, serverSpans(25, now - 1200n * SEC, 50), "isolation-test", ordersRes);
  await ingestSpans(orders, serverSpans(25, now - 150n * SEC, 400), "isolation-test", ordersRes);
  await ingestSpans(inventory, serverSpans(25, now - 150n * SEC, 5), "isolation-test", { "service.instance.id": `${inventory}-1`, "container.id": inventoryCtr });

  const errors = collectErrors(page);
  await login(page, "isolation@karbird.local", WORKSPACE);
  await expect(page.getByText("Starting engine…")).toBeHidden();

  const toggle = page.getByRole("checkbox", { name: "Infrastructure" });
  await expect(toggle).not.toBeChecked();
  await expect(page.getByRole("region", { name: "Machines" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Pause live" })).toBeVisible();
  await toggle.check();

  const machines = page.getByRole("region", { name: "Machines" });
  await expect(machines.getByRole("heading", { name: "Machines" })).toBeVisible();
  const machineButton = machines.getByRole("button", { name: `${host} · CPU 95% · RAM 870/1024 MB` });
  // The snapshot follows the cursor; processing takes a few seconds, so reload until it is there.
  await expect
    .poll(
      async () => {
        if ((await machineButton.count()) > 0) return true;
        await page.reload();
        await expect(page.getByText("Starting engine…")).toBeHidden();
        await page.getByRole("checkbox", { name: "Infrastructure" }).check();
        return false;
      },
      { timeout: 60_000, intervals: [3_000] },
    )
    .toBe(true);

  // Deterministic noisy-neighbour hint on the map.
  const banner = page.getByRole("status", { name: "Noisy neighbour" }).filter({ hasText: orders });
  await expect(banner).toBeVisible({ timeout: 30_000 });
  await expect(banner).toContainText(`${inventory} used 95% of CPU on ${host}`);
  await banner.getByRole("button", { name: "Show" }).click();

  // Service → instance → container → machine.
  const inspector = page.getByRole("complementary", { name: "Inspector" });
  await expect(inspector.getByRole("heading", { name: orders, exact: true })).toBeVisible();
  const runsOn = inspector.getByRole("region", { name: "Runs on" });
  await expect(runsOn).toContainText("1 instance");
  await expect(runsOn).toContainText(`${orders}-1`);
  await expect(runsOn).toContainText(`container ${orders}`);
  await expect(runsOn.getByRole("list", { name: "Noisy neighbour" })).toContainText(inventory);
  await runsOn.getByRole("button", { name: host, exact: true }).click();

  // Machine card: metrics and containers with their shares.
  await expect(inspector.getByRole("heading", { name: host, exact: true })).toBeVisible();
  await expect(machineButton).toHaveAttribute("aria-pressed", "true");
  const metrics = inspector.getByLabel("Machine metrics");
  await expect(metrics).toContainText("CPU");
  await expect(metrics).toContainText("95%");
  await expect(metrics).toContainText("Disk");
  await expect(metrics).toContainText("Network");
  const containers = inspector.getByRole("region", { name: "Containers" });
  await expect(containers.getByRole("heading", { name: "Containers (2)" })).toBeVisible();
  await expect(containers.getByLabel(`${inventory} CPU share`)).toHaveText("CPU 95%");
  await expect(containers.getByLabel(`${orders} CPU share`)).toHaveText("CPU 2%");
  await page.screenshot({ path: "../../test-results/e2e/infrastructure-layer.png" });

  // Traces of the same moment for the machine's busiest container: time stops, the Signals tab opens.
  await containers.getByRole("group", { name: `Signals for ${inventory}` }).getByRole("button", { name: "Traces at this moment" }).click();
  await expect(page.getByRole("tab", { name: "Signals" })).toHaveAttribute("aria-selected", "true");
  const signals = page.getByRole("region", { name: "Signals" });
  await expect(signals.getByRole("tab", { name: "Traces" })).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("button", { name: "Pause live" })).toHaveCount(0);

  // Logs from the machine card itself.
  await page.getByRole("tab", { name: "Details" }).click();
  await inspector.getByRole("group", { name: `Signals for ${host}` }).getByRole("button", { name: "Logs at this moment" }).click();
  await expect(signals.getByRole("tab", { name: "Logs" })).toHaveAttribute("aria-selected", "true");

  // Switching the layer off hides machines again.
  await toggle.uncheck();
  await expect(page.getByRole("region", { name: "Machines" })).toHaveCount(0);
  expect(errors).toEqual([]);
});
