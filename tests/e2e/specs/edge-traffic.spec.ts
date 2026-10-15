import { expect, test } from "./isolated-test";
import { mapLayer, collectErrors, ingestSpans, login, randomHex, type SpanInput } from "./helpers";

const WORKSPACE = "/app/isolation/isolation-test";
const SEC = 1_000_000_000n;
const MS = 1_000_000n;
const MB = 1024 * 1024;

test("a link between two components shows its calls and data flow, from the inspector and from the map", async ({ page }) => {
  // Includes a backend refresh plus multiple inspector/renderer transitions.
  test.setTimeout(120_000);
  const errors = collectErrors(page);
  await login(page, "isolation@karbird.local", WORKSPACE);
  await expect(page.getByText("Starting engine…")).toBeHidden();
  const run = randomHex(4);
  const gw = `et-gw-${run}`;
  const orders = `et-orders-${run}`;
  const db = `et_${run}`;
  const dbNode = `postgresql: ${db}`;
  const now = BigInt(Date.now()) * 1_000_000n;
  const activeUntil = now + 10n * 60n * SEC;

  // Four requests gw → orders, each answered with 3 MB; gw also queries PostgreSQL (no sizes).
  const gwSpans: SpanInput[] = [];
  const orderSpans: SpanInput[] = [];
  for (let i = 0n; i < 4n; i++) {
    const [traceId, root, client] = [randomHex(16), randomHex(8), randomHex(8)];
    const t = now - 40n * SEC + i * 10n * SEC;
    gwSpans.push(
      { traceId, spanId: root, name: "GET /checkout", kind: 2, startNs: t, endNs: activeUntil },
      { traceId, spanId: client, parentSpanId: root, name: "POST", kind: 3, startNs: t + MS, endNs: activeUntil },
      {
        traceId,
        spanId: randomHex(8),
        parentSpanId: root,
        name: "SELECT",
        kind: 3,
        startNs: t + 51n * MS,
        endNs: activeUntil,
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
      endNs: activeUntil,
      attributes: { "http.request.body.size": "2048", "http.response.body.size": String(3 * MB) },
    });
  }
  await ingestSpans(gw, gwSpans);
  await ingestSpans(orders, orderSpans);

  await page.getByRole("button", { name: "Pause live" }).click();
  const zoomToDetail = async () => {
    const timeline = page.getByLabel("Timeline");
    for (let i = 0; i < 2; i++) await timeline.getByRole("button", { name: "Zoom in time" }).click();
  };
  await zoomToDetail();
  const tableView = await mapLayer(page, "Table view");
  await tableView.check();
  await page.getByRole("button", { name: "Map layers" }).click();
  const gwRow = page.getByRole("table").getByRole("button", { name: gw, exact: true });
  // The processor may still be committing the spans; keep the live page mounted so
  // its WebSocket frame can pick them up without resetting the selected time.
  await expect(gwRow).toBeVisible({ timeout: 60_000 });

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
  await expect(
    flow.getByText(
      "Request size known for 4 of 4 calls; response size known for 4 of 4 calls. Rates include only calls with a reported size for that direction.",
    ),
  ).toBeVisible();
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
  await (await mapLayer(page, "Table view")).uncheck();
  const map = page.getByRole("application", { name: /Service map/ });
  await expect(map).toHaveAttribute("data-camera", /scale/);
  const mapText = page.getByRole("region", { name: "System Map text" });
  await expect(mapText.getByText(gw, { exact: true })).toBeAttached();
  await expect(mapText.getByText(orders, { exact: true })).toBeAttached();
  // Shared incremental geometry preserves the camera: explicitly include the new cards
  // before testing a click on their line, as a user would do after leaving the table.
  await map.focus();
  await page.keyboard.press("0");
  const mapKey = page.getByRole("button", { name: "Show map key" });
  await mapKey.hover();
  await expect(page.getByRole("dialog", { name: "Map key" })).toHaveCount(0);
  await mapKey.focus();
  await expect(page.getByRole("dialog", { name: "Map key" })).toHaveCount(0);
  await mapKey.press("Enter");
  await expect(page.getByRole("dialog", { name: "Map key" })).toBeVisible();
  await page.getByRole("button", { name: "Close map key" }).press("Enter");
  await expect(page.getByRole("dialog", { name: "Map key" })).toHaveCount(0);
  const webgl = await page
    .locator("main canvas")
    .or(page.getByText(/WebGL is unavailable/))
    .first()
    .evaluate((el) => el.tagName === "CANVAS");
  test.skip(!webgl, "WebGL is unavailable in this browser: the map falls back to the table view");
  await expect(map).toHaveAttribute("data-selected-edge", /^\d+$/);
  await expect(map).toHaveAttribute("data-selection-dimmed", "true");
  const index = await map.getAttribute("data-selected-edge");
  const anchor = async () => {
    const all = (await map.getAttribute("data-edge-anchors")) ?? "";
    const hit = all.split(";").find((a) => a.startsWith(`${index}:`));
    return hit ? hit.slice(hit.indexOf(":") + 1).split(",").map(Number) : null;
  };
  await expect.poll(anchor).not.toBeNull();
  await link.getByRole("button", { name: "Close link" }).press("Enter");
  await expect(map).toHaveAttribute("data-selected-edge", "");
  await expect(map).toHaveAttribute("data-selection-dimmed", "false");
  await expect(link).toHaveCount(0);

  const [ax, ay] = (await anchor()) ?? [0, 0];
  const box = await map.boundingBox();
  if (!box) throw new Error("map has no box");
  await page.mouse.click(box.x + (ax ?? 0), box.y + (ay ?? 0));
  await expect(link.getByRole("heading", { name: `${gw} → ${orders}` })).toBeVisible();
  await expect(map).toHaveAttribute("data-selected-edge", index ?? "");
  await expect(map).toHaveAttribute("data-selection-dimmed", "true");
  await page.mouse.click(box.x + (ax ?? 0), box.y + (ay ?? 0));
  await expect(link).toHaveCount(0);
  await expect(map).toHaveAttribute("data-selected-edge", "");
  await expect(map).toHaveAttribute("data-selection-dimmed", "false");
  await page.screenshot({ path: "../../test-results/e2e/edge-traffic.png" });
  expect(errors).toEqual([]);
});
