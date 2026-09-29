import { expect, test } from "@playwright/test";
import { login } from "./helpers";

const WORKSPACE = "/app/demo/local-demo";

test.describe("workspace on demo data", () => {
  test.beforeEach(async ({ page }) => {
    await login(page, "admin@controlhub.local", WORKSPACE);
    await expect(page).toHaveURL(new RegExp(`${WORKSPACE}$`));
  });

  test("engine starts in a worker and the map shows demo services", async ({ page }) => {
    await expect(page.getByText("DEMO", { exact: true })).toBeVisible();
    await expect(page.getByText("Starting engine…")).toBeHidden();
    await expect(page.getByText(/Engine failed/)).toHaveCount(0);
    // The live window summary must reflect loaded data (regression: stale throttled summary).
    await expect(page.getByLabel("Timeline")).toContainText(/[1-9]\d* req/);

    await page.getByRole("button", { name: "Engine" }).click();
    await expect(page.getByText("WASM init")).toBeVisible();
    await page.getByRole("button", { name: "Engine" }).click();

    // Deterministic check through the accessible table view (same engine frames).
    await page.getByRole("checkbox", { name: "Table view" }).check();
    const table = page.getByRole("table");
    await expect(table.getByRole("button", { name: "gateway", exact: true })).toBeVisible();
    await expect(table.getByRole("button", { name: "orders", exact: true })).toBeVisible();

    await table.getByRole("button", { name: "orders", exact: true }).click();
    const inspector = page.getByRole("complementary", { name: "Inspector" });
    await expect(inspector.getByRole("heading", { name: "orders", exact: true })).toBeVisible();
    await expect(inspector.getByText(/observed|inferred/).first()).toBeVisible();
  });

  test("map renders on a canvas when WebGL is available", async ({ page }) => {
    const canvas = page.locator("main canvas");
    const fallback = page.getByText(/WebGL is unavailable/);
    await expect(canvas.or(fallback)).toBeVisible();
    await page.screenshot({ path: "../../test-results/e2e/workspace.png" });
  });

  test("timeline modes: pause, replay, return to live", async ({ page }) => {
    const timeline = page.getByLabel("Timeline");
    await expect(timeline.getByText("live", { exact: true })).toBeVisible();
    await timeline.getByRole("button", { name: "Pause live" }).click();
    await expect(timeline.getByText("paused", { exact: true })).toBeVisible();

    await timeline.getByRole("button", { name: "Back 5 seconds" }).click();
    const cursor = timeline.getByLabel("Selected time");
    const before = await cursor.getAttribute("aria-valuetext");
    const prev = timeline.getByRole("button", { name: "Previous event" });
    await expect(prev).toBeEnabled();
    await prev.click();
    await expect(cursor).not.toHaveAttribute("aria-valuetext", before ?? "");
    const next = timeline.getByRole("button", { name: "Next event" });
    await expect(next).toBeEnabled();
    await next.click();
    await expect(timeline.getByText("paused", { exact: true })).toBeVisible();

    await timeline.getByLabel("Speed").selectOption("2");
    await timeline.getByRole("button", { name: "Play replay" }).click();
    await expect(timeline.getByText("replay", { exact: true })).toBeVisible();
    await timeline.getByRole("button", { name: "Pause", exact: true }).click();
    await expect(timeline.getByText("paused", { exact: true })).toBeVisible();

    await timeline.getByRole("button", { name: "Return to live" }).click();
    await expect(timeline.getByText("live", { exact: true })).toBeVisible();
  });

  test("live mode streams over WebSocket and survives pause and reload", async ({ page }) => {
    const transport = page.getByLabel("Live transport");
    await expect(transport).toHaveText("Live: WebSocket");
    const timeline = page.getByLabel("Timeline");
    await timeline.getByRole("button", { name: "Pause live" }).click();
    await expect(transport).toHaveCount(0);
    await timeline.getByRole("button", { name: "Return to live" }).click();
    await expect(transport).toHaveText("Live: WebSocket");

    await page.reload();
    await expect(page.getByText("Starting engine…")).toBeHidden();
    await expect(transport).toHaveText("Live: WebSocket");
    await expect(timeline).toContainText(/[1-9]\d* req/);
    // Pushed batches are merged into the timeline instead of rebuilding it.
    const engine = page.getByRole("button", { name: "Engine" });
    const merges = page.getByText(/Timeline rebuilds \/ merges/);
    await expect
      .poll(async () => {
        // Diagnostics are read when the panel opens.
        if (await merges.isVisible()) await engine.click();
        await engine.click();
        await merges.waitFor();
        const row = await merges.locator("..").innerText();
        return Number(/\/\s*(\d+)\s*$/.exec(row.trim())?.[1] ?? 0);
      }, { timeout: 20_000 })
      .toBeGreaterThan(0);
  });

  test("logs, traces waterfall and events panels load", async ({ page }) => {
    const panel = page.getByRole("region", { name: "Signals" });
    await expect(panel.getByLabel("Log records")).toBeVisible();
    await expect(panel.getByText(/order (loaded|created|not found)/).first()).toBeVisible();

    await panel.getByRole("tab", { name: "Traces" }).click();
    const traces = panel.getByRole("list", { name: "Traces" });
    await traces.getByRole("button").first().click();
    await expect(panel.getByRole("list", { name: "Span waterfall" })).toBeVisible();
    await expect(panel.getByText(/\d+ spans ·/)).toBeVisible();

    await panel.getByRole("tab", { name: "Events" }).click();
    await expect(panel.getByText(/request\.(started|completed)/).first()).toBeVisible();
  });
});

test("isolation user cannot open the demo environment", async ({ page }) => {
  await login(page, "isolation@controlhub.local", WORKSPACE);
  await expect(page.getByText(/does not exist or you do not have access/)).toBeVisible();
});
