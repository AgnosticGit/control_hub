import { expect, test } from "@playwright/test";
import { collectErrors, ingestSpans, login, randomHex, type SpanInput } from "./helpers";

const WORKSPACE = "/app/isolation/isolation-test";
const SEC = 1_000_000_000n;
const MS = 1_000_000n;
const MB = 1024 * 1024;

test("a link between two components shows its calls and data flow, from the inspector and from the map", async ({ page }) => {
  const run = randomHex(4);
  const gw = `et-gw-${run}`;
  const orders = `et-orders-${run}`;
  const db = `et_${run}`;
  const dbNode = `postgresql: ${db}`;
  const now = BigInt(Date.now()) * 1_000_000n;

  // Four requests gw → orders, each answered with 3 MB; gw also queries PostgreSQL (no sizes).
  const gwSpans: SpanInput[] = [];
  const orderSpans: SpanInput[] = [];
  for (let i = 0n; i < 4n; i++) {
    const [traceId, root, client] = [randomHex(16), randomHex(8), randomHex(8)];
    const t = now - 50n * SEC + i * 10n * SEC;
    gwSpans.push(
      { traceId, spanId: root, name: "GET /checkout", kind: 2, startNs: t, endNs: t + 60n * MS },
      { traceId, spanId: client, parentSpanId: root, name: "POST", kind: 3, startNs: t + MS, endNs: t + 50n * MS },
      {
        traceId,
        spanId: randomHex(8),
        parentSpanId: root,
        name: "SELECT",
        kind: 3,
        startNs: t + 51n * MS,
        endNs: t + 55n * MS,
        attributes: { "db.system": "postgresql", "db.name": db, "server.address": "localhost", "server.port": "5432" },
      },
    );
    orderSpans.push({
      traceId,
      spanId: randomHex(8),
      parentSpanId: client,
      name: "POST /orders",
      kind: 2,
      startNs: t + 2n * MS,
      endNs: t + 48n * MS,
      attributes: { "http.request.body.size": "2048", "http.response.body.size": String(3 * MB) },
    });
  }
  await ingestSpans(gw, gwSpans);
  await ingestSpans(orders, orderSpans);

  const errors = collectErrors(page);
  await login(page, "isolation@karbird.local", WORKSPACE);
  const tableView = page.getByRole("checkbox", { name: "Table view" });
  const gwRow = page.getByRole("table").getByRole("button", { name: gw, exact: true });
  // Processing and the topology refresh take a few seconds: reload until the service is listed.
  await expect
    .poll(
      async () => {
        await expect(page.getByText("Starting engine…")).toBeHidden();
        await tableView.check();
        if ((await gwRow.count()) > 0) return true;
        await page.reload();
        return false;
      },
      { timeout: 60_000, intervals: [3_000] },
    )
    .toBe(true);

  const inspector = page.getByRole("complementary", { name: "Inspector" });
  const relations = inspector.getByRole("region", { name: "Relations" });
  const link = inspector.getByRole("article", { name: "Link details" });
  await gwRow.click();
  await expect(inspector.getByRole("heading", { name: gw, exact: true })).toBeVisible();

  // Calls from the engine and the data flow from the API.
  await relations.getByRole("button", { name: `Open link ${gw} → ${orders}` }).click();
  await expect(link.getByRole("heading", { name: `${gw} → ${orders}` })).toBeVisible();
  await expect(link).toContainText("calls · observed");
  const calls = link.getByRole("region", { name: "Calls at selected time" });
  for (const label of ["Calls", "Calls/min", "In flight", "Errors", "Error rate", "p95"]) {
    await expect(calls.getByText(label, { exact: true })).toBeVisible();
  }
  const flow = link.getByRole("region", { name: "Data flow" });
  await expect(flow.getByText("Size known for all 4 calls.")).toBeVisible();
  const statValue = (label: string) => flow.getByText(label, { exact: true }).locator("xpath=following-sibling::dd");
  await expect(statValue("↓ Responses avg")).toHaveText(/^[\d.]+ KB\/s$/);
  await expect(statValue("↓ Responses peak")).toHaveText(/^[\d.]+ KB\/s$/);
  await expect(statValue("↑ Requests avg")).toHaveText(/^[\d.]+ B\/s$/);
  await expect(flow.getByRole("figure", { name: "Data flow over the loaded range" })).toContainText("peak at");

  // A database link counts calls but its services report no sizes: unknown, not zero.
  await link.getByRole("group", { name: "Link ends" }).getByRole("button", { name: gw }).click();
  await relations.getByRole("button", { name: `Open link ${gw} → ${dbNode}` }).click();
  await expect(link.getByRole("heading", { name: `${gw} → ${dbNode}` })).toBeVisible();
  await expect(link.getByRole("region", { name: "Data flow" })).toContainText("do not report request or response sizes");
  await expect(link.getByText("↓ Responses avg")).toHaveCount(0);

  // On the map: the open link is highlighted, and clicking the line opens it again.
  await link.getByRole("group", { name: "Link ends" }).getByRole("button", { name: gw }).click();
  await relations.getByRole("button", { name: `Open link ${gw} → ${orders}` }).click();
  await tableView.uncheck();
  const map = page.getByRole("application", { name: /Service map/ });
  const webgl = await page
    .locator("main canvas")
    .or(page.getByText(/WebGL is unavailable/))
    .first()
    .evaluate((el) => el.tagName === "CANVAS");
  test.skip(!webgl, "WebGL is unavailable in this browser: the map falls back to the table view");
  await expect(map).toHaveAttribute("data-selected-edge", /^\d+$/);
  const index = await map.getAttribute("data-selected-edge");
  const anchor = async () => {
    const all = (await map.getAttribute("data-edge-anchors")) ?? "";
    const hit = all.split(";").find((a) => a.startsWith(`${index}:`));
    return hit ? hit.slice(hit.indexOf(":") + 1).split(",").map(Number) : null;
  };
  await expect.poll(anchor).not.toBeNull();
  await link.getByRole("button", { name: "Close link" }).click();
  await expect(map).toHaveAttribute("data-selected-edge", "");
  await expect(link).toHaveCount(0);

  const [ax, ay] = (await anchor()) ?? [0, 0];
  const box = await map.boundingBox();
  if (!box) throw new Error("map has no box");
  await page.mouse.click(box.x + (ax ?? 0), box.y + (ay ?? 0));
  await expect(link.getByRole("heading", { name: `${gw} → ${orders}` })).toBeVisible();
  await expect(map).toHaveAttribute("data-selected-edge", index ?? "");
  await page.screenshot({ path: "../../test-results/e2e/edge-traffic.png" });
  expect(errors).toEqual([]);
});
