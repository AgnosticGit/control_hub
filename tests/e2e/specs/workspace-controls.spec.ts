import { expect, test, type Locator, type Page } from "@playwright/test";
import { collectErrors, login } from "./helpers";

const WORKSPACE = "/app/demo/local-demo";

async function openTable(page: Page): Promise<Locator> {
  await page.getByRole("checkbox", { name: "Table view" }).check();
  const table = page.getByRole("table");
  await expect(table.getByRole("button", { name: "gateway", exact: true })).toBeVisible();
  return table;
}

/** Text of one column for every body row of the node table. */
async function column(table: Locator, index: number): Promise<string[]> {
  return table.locator("tbody tr").evaluateAll((rows, i) => rows.map((r) => r.children[i]?.textContent?.trim() ?? ""), index);
}

test.describe("workspace controls on demo data", () => {
  test.beforeEach(async ({ page }) => {
    await login(page, "admin@controlhub.local", WORKSPACE);
    await expect(page.getByText("Starting engine…")).toBeHidden();
  });

  test("KPI row summarises services, request rate, error rate and active requests", async ({ page }) => {
    const kpi = (label: string) => page.locator("div.card").filter({ has: page.getByText(label, { exact: true }) }).first();
    await expect(kpi("Services")).toContainText(/Services\s*([7-9]|\d{2,})/);
    await expect(kpi("Requests / min")).toContainText(/Requests \/ min\s*[1-9]\d*/);
    await expect(kpi("Requests / min")).toContainText(/in the last 30 s/);
    await expect(kpi("Error rate")).toContainText(/\d+\.\d%/);
    await expect(kpi("Active requests")).toContainText("in flight at the selected time");
  });

  test("Find component filters the map to matching names (case-insensitive) and clears", async ({ page }) => {
    const table = await openTable(page);
    const search = page.getByRole("searchbox", { name: "Find component on map" });
    await search.fill("ORDE");
    await expect(table.getByRole("button", { name: "gateway", exact: true })).toHaveCount(0);
    await expect(table.getByRole("button", { name: "orders", exact: true })).toBeVisible();
    expect((await column(table, 0)).every((n) => n.toLowerCase().includes("orde"))).toBe(true);

    await search.fill("no-such-component-xyz");
    await expect(page.getByText("No components match the current filters.")).toBeVisible();

    await search.fill("");
    await expect(table.getByRole("button", { name: "gateway", exact: true })).toBeVisible();
    expect((await column(table, 0)).length).toBeGreaterThanOrEqual(7);
  });

  test("Hide inferred removes inferred dependencies; Only problems keeps degraded/failing only", async ({ page }) => {
    const table = await openTable(page);
    await expect.poll(async () => (await column(table, 5)).filter((p) => p === "inferred").length).toBeGreaterThan(0);
    await page.getByRole("checkbox", { name: "Hide inferred" }).check();
    await expect.poll(async () => (await column(table, 5)).filter((p) => p === "inferred").length).toBe(0);
    await expect(table.getByRole("button", { name: "orders", exact: true })).toBeVisible();
    await page.getByRole("checkbox", { name: "Hide inferred" }).uncheck();
    await expect.poll(async () => (await column(table, 5)).filter((p) => p === "inferred").length).toBeGreaterThan(0);

    await page.getByRole("checkbox", { name: "Only problems" }).check();
    const empty = page.getByText("No components match the current filters.");
    await expect
      .poll(async () => {
        if (await empty.isVisible()) return true;
        const statuses = await column(page.getByRole("table"), 2);
        return statuses.every((s) => s === "Degraded" || s === "Failing");
      })
      .toBe(true);
    await page.getByRole("checkbox", { name: "Only problems" }).uncheck();
    await expect(page.getByRole("table").getByRole("button", { name: "gateway", exact: true })).toBeVisible();
  });

  test("table rows are ordered by status then name", async ({ page }) => {
    const table = await openTable(page);
    const rank: Record<string, number> = { Failing: 3, Degraded: 2, Healthy: 1, "No data": 0 };
    await expect
      .poll(async () => {
        const names = await column(table, 0);
        const statuses = await column(table, 2);
        for (let i = 1; i < names.length; i++) {
          const a = rank[statuses[i - 1] ?? ""] ?? 0;
          const b = rank[statuses[i] ?? ""] ?? 0;
          if (a < b || (a === b && (names[i - 1] ?? "").localeCompare(names[i] ?? "") > 0)) return false;
        }
        return names.length > 0;
      })
      .toBe(true);
  });

  test("Inspector shows metrics and relations, navigates along relations and closes", async ({ page }) => {
    const table = await openTable(page);
    await table.getByRole("button", { name: "orders", exact: true }).click();
    const inspector = page.getByRole("complementary", { name: "Inspector" });
    await expect(page.getByRole("tab", { name: "Details" })).toHaveAttribute("aria-selected", "true");
    await expect(inspector.getByRole("heading", { name: "orders", exact: true })).toBeVisible();
    await expect(inspector).toContainText("service · observed");
    const metrics = inspector.getByRole("region", { name: "Metrics at selected time" });
    for (const label of ["Active now", "Requests", "Completed", "Errors", "Avg latency", "Logs", "Error logs", "Events"]) {
      await expect(metrics.getByText(label, { exact: true })).toBeVisible();
    }
    await expect(metrics).toContainText(/healthy|degraded|failing|unknown/);
    await expect(metrics).toContainText("30 s window");

    const relations = inspector.getByRole("region", { name: "Relations" });
    await expect(relations.getByRole("button", { name: "gateway", exact: true })).toBeVisible();
    // The inferred database dependency lists why it was inferred.
    await expect(relations.getByText("inferred").first()).toBeVisible();
    await expect(relations).toContainText("semantic-convention attributes");

    await relations.getByRole("button", { name: "gateway", exact: true }).click();
    await expect(inspector.getByRole("heading", { name: "gateway", exact: true })).toBeVisible();
    await inspector.getByRole("button", { name: "Close inspector" }).click();
    await expect(inspector.getByText("Select a component on the map to inspect it.")).toBeVisible();
  });

  test("map zoom and fit buttons work without errors", async ({ page }) => {
    const errors = collectErrors(page);
    const map = page.getByRole("application", { name: /Service map/ });
    test.skip((await page.getByText(/WebGL is unavailable/).count()) > 0, "no WebGL in this browser");
    await expect(map).toBeVisible();
    for (const name of ["Zoom in", "Zoom in", "Zoom out", "Fit to screen"]) {
      await page.getByRole("button", { name, exact: true }).click();
    }
    await map.focus();
    for (const key of ["+", "-", "ArrowLeft", "ArrowUp", "0", "Escape"]) await page.keyboard.press(key);
    await expect(page.locator("main canvas").first()).toBeVisible();
    expect(errors.filter((e) => !/Download the React DevTools/.test(e))).toEqual([]);
  });

  test("side panel splitter resizes with the keyboard, is clamped and remembered", async ({ page }) => {
    const splitter = page.getByRole("separator", { name: "Resize side panel" });
    const start = Number(await splitter.getAttribute("aria-valuenow"));
    await splitter.focus();
    await page.keyboard.press("ArrowLeft");
    await expect(splitter).toHaveAttribute("aria-valuenow", String(start + 24));
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("ArrowRight");
    await expect(splitter).toHaveAttribute("aria-valuenow", String(Math.max(320, start - 24)));
    for (let i = 0; i < 30; i++) await page.keyboard.press("ArrowLeft");
    await expect(splitter).toHaveAttribute("aria-valuenow", await splitter.getAttribute("aria-valuemax") ?? "760");
    await page.reload();
    const again = page.getByRole("separator", { name: "Resize side panel" });
    await expect(again).toHaveAttribute("aria-valuenow", "760");
  });

  test("logs: text search, severity filters and the per-service toggle", async ({ page }) => {
    const panel = page.getByRole("region", { name: "Signals" });
    const records = panel.getByLabel("Log records");
    await expect(records).toBeVisible();
    const rows = records.locator(":scope > div > div");
    const cells = (i: number) => rows.evaluateAll((els, n) => els.map((e) => e.children[n]?.textContent?.trim() ?? ""), i);

    await panel.getByLabel("Search log text").fill("order");
    await expect.poll(async () => (await cells(3)).length > 0 && (await cells(3)).every((b) => /order/i.test(b))).toBe(true);
    await panel.getByLabel("Search log text").fill("");

    const severity = panel.getByRole("group", { name: "Minimum severity" });
    await expect(severity.getByRole("button", { name: "All" })).toHaveAttribute("aria-pressed", "true");
    await severity.getByRole("button", { name: "Warn+" }).click();
    await expect(severity.getByRole("button", { name: "Warn+" })).toHaveAttribute("aria-pressed", "true");
    await expect(severity.getByRole("button", { name: "All" })).toHaveAttribute("aria-pressed", "false");
    await expect.poll(async () => (await cells(1)).every((s) => !/^(info|debug|trace)/i.test(s))).toBe(true);
    await severity.getByRole("button", { name: "Error+" }).click();
    await expect.poll(async () => (await cells(1)).every((s) => /^(error|fatal|1[7-9]|2\d)/i.test(s))).toBe(true);
    await severity.getByRole("button", { name: "All" }).click();

    // Selecting a service narrows the panel to it; the toggle widens it again.
    const table = await openTable(page);
    await table.getByRole("button", { name: "orders", exact: true }).click();
    await page.getByRole("tab", { name: "Signals" }).click();
    const only = panel.getByRole("checkbox", { name: "Only orders" });
    await expect(only).toBeChecked();
    await expect.poll(async () => {
      const s = await cells(2);
      return s.length > 0 && s.every((x) => x === "orders");
    }).toBe(true);
    await only.uncheck();
    await expect.poll(async () => new Set(await cells(2)).size).toBeGreaterThan(1);
  });

  test("a trace link in the logs opens the trace waterfall; span details show attributes", async ({ page }) => {
    const panel = page.getByRole("region", { name: "Signals" });
    const records = panel.getByLabel("Log records");
    await records.getByRole("button", { name: "trace" }).first().click();
    await expect(panel.getByRole("tab", { name: "Traces" })).toHaveAttribute("aria-selected", "true");
    const waterfall = panel.getByRole("list", { name: "Span waterfall" });
    await expect(waterfall).toBeVisible();
    await expect(panel.getByText(/\d+ spans ·/)).toBeVisible();
    await expect(panel.getByText(/^[0-9a-f]{32}$/)).toBeVisible();

    await waterfall.getByRole("button").first().click();
    const detail = panel.locator("dl").first();
    await expect(detail).toBeVisible();
    await expect(detail.locator("dt").first()).toBeVisible();
    await expect(detail).not.toContainText(/authorization|cookie|db\.statement/i);
  });

  test("traces list marks the selected trace and filters to errors", async ({ page }) => {
    const panel = page.getByRole("region", { name: "Signals" });
    await panel.getByRole("tab", { name: "Traces" }).click();
    await expect(panel.getByText("Select a trace.")).toBeVisible();
    const list = panel.getByRole("list", { name: "Traces" });
    const first = list.getByRole("button").first();
    await first.click();
    await expect(first).toHaveAttribute("aria-current", "true");
    await panel.getByLabel("Only errors").check();
    await expect.poll(async () => {
      const n = await list.getByRole("button").count();
      const ok = await list.getByLabel("ok", { exact: true }).count();
      return n === 0 || ok === 0;
    }).toBe(true);
  });

  test("events tab lists typed events and opens linked traces", async ({ page }) => {
    const panel = page.getByRole("region", { name: "Signals" });
    await panel.getByRole("tab", { name: "Events" }).click();
    await expect(panel.getByRole("tab", { name: "Events" })).toHaveAttribute("aria-selected", "true");
    const table = panel.getByRole("table");
    await expect(table.getByRole("columnheader", { name: "Type" })).toBeVisible();
    await expect(table.getByText(/request\.(started|completed)/).first()).toBeVisible();
    const open = table.getByRole("button", { name: "open" }).first();
    if ((await open.count()) > 0) {
      await open.click();
      await expect(panel.getByRole("tab", { name: "Traces" })).toHaveAttribute("aria-selected", "true");
      await expect(panel.getByRole("list", { name: "Span waterfall" })).toBeVisible();
    }
  });

  test("top-bar presets load history (HISTORY mode) and Return to live goes back", async ({ page }) => {
    const timeline = page.getByLabel("Timeline");
    await expect(timeline.getByText("live", { exact: true })).toBeVisible();
    await page.getByRole("group", { name: "Load a recent range" }).getByRole("button", { name: "1h" }).click();
    await expect(timeline.getByText("historical", { exact: true })).toBeVisible();
    await expect(page.getByText("HISTORY", { exact: true })).toBeVisible();
    await expect(page.getByLabel("Live transport")).toHaveCount(0);
    await expect(timeline).toContainText(/1h|60m|1 h/);
    await page.getByRole("group", { name: "Load a recent range" }).getByRole("button", { name: "6h" }).click();
    await expect(timeline.getByText("historical", { exact: true })).toBeVisible();
    await timeline.getByRole("button", { name: "Return to live" }).click();
    await expect(timeline.getByText("live", { exact: true })).toBeVisible();
    await expect(page.getByLabel("Live transport")).toHaveText("Live: WebSocket");
  });

  test("Range… validates the range and loads a valid one", async ({ page }) => {
    const timeline = page.getByLabel("Timeline");
    const toggle = timeline.getByRole("button", { name: "Range…" });
    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-expanded", "true");
    const from = timeline.getByLabel("From", { exact: true });
    const to = timeline.getByLabel("To", { exact: true });
    const load = timeline.getByRole("button", { name: "Load range" });
    const alert = timeline.getByRole("alert");

    await from.fill("2026-09-20T12:00");
    await to.fill("2026-09-20T11:00");
    await load.click();
    await expect(alert).toHaveText("Choose a valid range (from before to).");

    await from.fill("2026-09-10T12:00");
    await to.fill("2026-09-20T12:00");
    await load.click();
    await expect(alert).toHaveText("Range may not exceed 7 days.");

    await expect(timeline.getByRole("button", { name: "Last 6 h" })).toBeVisible();
    const now = new Date(Date.now() - new Date().getTimezoneOffset() * 60_000);
    const hourAgo = new Date(now.getTime() - 3_600_000);
    await from.fill(hourAgo.toISOString().slice(0, 16));
    await to.fill(now.toISOString().slice(0, 16));
    await load.click();
    await expect(load).toHaveCount(0);
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(timeline.getByText("historical", { exact: true })).toBeVisible();

    await toggle.click();
    await timeline.getByRole("button", { name: "Last 1 h" }).click();
    await expect(timeline.getByRole("button", { name: "Load range" })).toHaveCount(0);
    await expect(timeline.getByText("historical", { exact: true })).toBeVisible();
  });

  test("Mark A and Compare A ↔ now show a comparison in Details", async ({ page }) => {
    const timeline = page.getByLabel("Timeline");
    await timeline.getByRole("button", { name: "Pause live" }).click();
    await timeline.getByRole("button", { name: "Mark A" }).click();
    await expect(timeline.getByText(/^A: \d{2}:\d{2}:\d{2}/)).toBeVisible();
    for (let i = 0; i < 6; i++) await timeline.getByRole("button", { name: "Back 5 seconds" }).click();
    await timeline.getByRole("button", { name: "Compare A ↔ now" }).click();

    await expect(page.getByRole("tab", { name: "Details" })).toHaveAttribute("aria-selected", "true");
    const inspector = page.getByRole("complementary", { name: "Inspector" });
    await expect(inspector.getByRole("heading", { name: /^Compare .+ ↔ .+/ })).toBeVisible();
    const diffs = inspector.getByRole("listitem");
    await expect(diffs.first().or(inspector.getByText("No component changed status or traffic."))).toBeVisible();
    if ((await diffs.count()) > 0) {
      await expect(diffs.first()).toContainText("Status");
      await expect(diffs.first()).toContainText("Requests");
    }
    await inspector.getByRole("button", { name: "Close comparison" }).click();
    await expect(inspector.getByRole("heading", { name: /^Compare/ })).toHaveCount(0);

    await timeline.getByRole("button", { name: "Clear mark A" }).click();
    await expect(timeline.getByRole("button", { name: "Mark A" })).toBeVisible();
  });

  test("the ruler supports Home, End, PageUp/PageDown and Shift+Arrow", async ({ page }) => {
    const timeline = page.getByLabel("Timeline");
    await timeline.getByRole("button", { name: "Pause live" }).click();
    const ruler = timeline.getByRole("slider", { name: "Selected time" });
    await ruler.focus();
    await page.keyboard.press("Home");
    await expect(ruler).toHaveAttribute("aria-valuenow", "0");
    await page.keyboard.press("End");
    await expect(ruler).toHaveAttribute("aria-valuenow", "100");
    // A tenth of the view back; the view is the data plus the 1/9 after "now" that has not happened yet.
    await page.keyboard.press("PageUp");
    await expect.poll(async () => Number(await ruler.getAttribute("aria-valuenow"))).toBeGreaterThanOrEqual(88);
    await expect.poll(async () => Number(await ruler.getAttribute("aria-valuenow"))).toBeLessThanOrEqual(90);
    await page.keyboard.press("PageDown");
    await expect(ruler).toHaveAttribute("aria-valuenow", "100");
    await page.keyboard.press("Shift+ArrowLeft");
    await expect.poll(async () => Number(await ruler.getAttribute("aria-valuenow"))).toBeLessThanOrEqual(91);
    await expect(timeline.getByText("paused", { exact: true })).toBeVisible();
  });

  test("Escape on the timeline clears a replay range", async ({ page }) => {
    const timeline = page.getByLabel("Timeline");
    const rows = timeline.locator('[data-area="rows"]');
    const box = await rows.boundingBox();
    expect(box).not.toBeNull();
    if (!box) return;
    await page.mouse.move(box.x + box.width * 0.4, box.y + 30);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width * 0.5, box.y + 30, { steps: 10 });
    await page.mouse.up();
    const chip = timeline.getByLabel("Replay range", { exact: true });
    await expect(chip).toBeVisible();
    await timeline.getByRole("slider", { name: "Selected time" }).focus();
    await page.keyboard.press("Escape");
    await expect(chip).toHaveCount(0);
  });

  test("time zoom out/in buttons change the visible span and Whole range restores it", async ({ page }) => {
    const timeline = page.getByLabel("Timeline");
    await timeline.getByRole("button", { name: "Pause live" }).click();
    const whole = timeline.getByRole("button", { name: "Whole range" });
    const span = whole.locator("xpath=following-sibling::span[1]");
    const full = await span.textContent();
    await timeline.getByRole("button", { name: "Zoom in time" }).click();
    await timeline.getByRole("button", { name: "Zoom in time" }).click();
    await expect(span).not.toHaveText(full ?? "");
    await expect(whole).toHaveAttribute("aria-pressed", "false");
    await timeline.getByRole("button", { name: "Zoom out time" }).click();
    await timeline.getByRole("button", { name: "Zoom out time" }).click();
    await timeline.getByRole("button", { name: "Zoom out time" }).click();
    // Zooming out is bounded by the loaded range.
    await expect(span).toHaveText(full ?? "");
    await whole.click();
    await expect(whole).toHaveAttribute("aria-pressed", "true");
  });
});
