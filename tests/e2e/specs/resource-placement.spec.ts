import { expect, test } from "@playwright/test";
import { mapLayer, collectErrors, ingestMetrics, ingestSpans, login, randomHex, type SpanInput } from "./helpers";

const WORKSPACE = "/app/isolation/isolation-test";
const MIB = 1024 * 1024;
const SEC = 1_000_000_000n;
const MS = 1_000_000n;

test("a database in a container on our machine runs there; a managed one is external", async ({ page }) => {
  const run = randomHex(4);
  const host = `e2e-rp-${run}`;
  const orders = `rp-orders-${run}`;
  const compose = `pg-${run}`;
  const ownDb = `orders_${run}`;
  const managedDb = `billing_${run}`;
  const managed = `rc1a-${run}.mdb.yandexcloud.net`;
  const [ordersCtr, pgCtr] = [randomHex(32), randomHex(32)];
  const now = BigInt(Date.now()) * 1_000_000n;

  const rss = (mb: number) => ({ name: "container.memory.usage.total", unit: "By", kind: "gauge" as const, points: [{ value: mb * MIB, timeNs: now - 20n * SEC }] });
  await ingestMetrics([
    {
      attributes: { "host.name": host, "host.id": host },
      metrics: [{ name: "system.cpu.logical.count", unit: "{cpu}", kind: "gauge", points: [{ value: 2, timeNs: now - 20n * SEC }] }],
    },
    { attributes: { "host.name": host, "host.id": host, "container.id": ordersCtr, "container.name": `/${orders}` }, metrics: [rss(120)] },
    {
      attributes: {
        "host.name": host,
        "host.id": host,
        "container.id": pgCtr,
        "container.name": `/${compose}-1`,
        "container.image.name": "postgres:16-alpine",
        "docker.compose.service": compose,
      },
      metrics: [rss(96)],
    },
  ]);

  // Requests to orders, each querying our own database (by Compose service name) and a managed one.
  const spans: SpanInput[] = Array.from({ length: 4 }, (_, i) => {
    const traceId = randomHex(16);
    const root = randomHex(8);
    const t = now - 60n * SEC + BigInt(i) * SEC;
    const db = (name: string, address: string, at: bigint): SpanInput => ({
      traceId,
      spanId: randomHex(8),
      parentSpanId: root,
      name: "SELECT",
      kind: 3,
      startNs: at,
      endNs: at + 3n * MS,
      attributes: { "db.system": "postgresql", "db.name": name, "server.address": address, "server.port": "5432" },
    });
    return [
      { traceId, spanId: root, name: "GET /orders", kind: 2, startNs: t, endNs: t + 20n * MS, attributes: { "http.request.method": "GET" } },
      db(ownDb, compose, t + 2n * MS),
      db(managedDb, managed, t + 8n * MS),
    ];
  }).flat();
  await ingestSpans(orders, spans, "isolation-test", { "service.instance.id": `${orders}-1`, "container.id": ordersCtr });

  const errors = collectErrors(page);
  await login(page, "isolation@karbird.local", WORKSPACE);
  await expect(page.getByText("Starting engine…")).toBeHidden();
  await (await mapLayer(page, "Infrastructure")).check();
  const machines = page.getByRole("region", { name: "Machines" });
  const machine = machines.getByRole("button", { name: new RegExp(`^${host}`) });
  const inspector = page.getByRole("complementary", { name: "Inspector" });
  const containers = inspector.getByRole("region", { name: "Containers" });
  const dbLink = containers.getByRole("button", { name: `postgresql: ${ownDb}`, exact: true });
  // Processing and the topology refresh take a few seconds: reload until the database is linked.
  await expect
    .poll(
      async () => {
        if ((await machine.count()) > 0) {
          await machine.click();
          if ((await dbLink.count()) > 0) return true;
        }
        await page.reload();
        await expect(page.getByText("Starting engine…")).toBeHidden();
        await (await mapLayer(page, "Infrastructure")).check();
        return false;
      },
      { timeout: 60_000, intervals: [3_000] },
    )
    .toBe(true);

  // The machine card shows the container's image and which database runs in it.
  await expect(containers).toContainText("postgres:16-alpine");
  await dbLink.click();

  await expect(inspector.getByRole("heading", { name: `postgresql: ${ownDb}`, exact: true })).toBeVisible();
  const runsOn = inspector.getByRole("region", { name: "Runs on" });
  await expect(runsOn.getByText("Internal", { exact: true })).toBeVisible();
  await expect(runsOn).toContainText(`container ${compose}-1`);
  await expect(runsOn).toContainText("Docker Compose service");
  await expect(runsOn.getByLabel("Addresses services connect to")).toHaveText(`${compose}:5432`);

  // Through the caller to the managed database: external, no machine invented.
  await inspector.getByRole("region", { name: "Relations" }).getByRole("button", { name: orders, exact: true }).click();
  await expect(inspector.getByRole("heading", { name: orders, exact: true })).toBeVisible();
  await inspector.getByRole("region", { name: "Relations" }).getByRole("button", { name: `postgresql: ${managedDb}`, exact: true }).click();
  await expect(inspector.getByRole("heading", { name: `postgresql: ${managedDb}`, exact: true })).toBeVisible();
  await expect(runsOn).toContainText("External: no Karbird agent there");
  await expect(runsOn).toContainText(managed);
  await expect(runsOn.getByRole("button", { name: host })).toHaveCount(0);

  // Back to our database, then to its machine from the inspector.
  await inspector.getByRole("region", { name: "Relations" }).getByRole("button", { name: orders, exact: true }).click();
  await inspector.getByRole("region", { name: "Relations" }).getByRole("button", { name: `postgresql: ${ownDb}`, exact: true }).click();
  await runsOn.getByRole("button", { name: host, exact: true }).click();
  await expect(inspector.getByRole("heading", { name: host, exact: true })).toBeVisible();
  await expect(machine).toHaveAttribute("aria-pressed", "true");
  await page.screenshot({ path: "../../test-results/e2e/resource-placement.png" });
  expect(errors).toEqual([]);
});
