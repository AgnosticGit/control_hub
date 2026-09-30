import { expect, test } from "@playwright/test";
import { collectErrors, login } from "./helpers";

const WORKSPACE = "/app/demo/local-demo";
/** Outside a fixed historical range the last 72 px of the timeline are the future stretch. */
const FUTURE_PX = 72;
/** Slider value (0 = start of the loaded data, 100 = now) at a share of a `width` px timeline. */
const dataPercent = (share: number, width: number) => ((share * width) / (width - FUTURE_PX)) * 100;

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

  test("map line style can be switched and is remembered", async ({ page }) => {
    const styles = page.getByRole("group", { name: "Line style" });
    await expect(styles.getByRole("button", { name: "Curved lines" })).toHaveAttribute("aria-pressed", "true");
    await styles.getByRole("button", { name: "Square lines" }).click();
    await expect(styles.getByRole("button", { name: "Square lines" })).toHaveAttribute("aria-pressed", "true");
    await page.reload();
    await expect(page.getByRole("group", { name: "Line style" }).getByRole("button", { name: "Square lines" })).toHaveAttribute(
      "aria-pressed",
      "true",
      { timeout: 30_000 },
    );
    await page.getByRole("group", { name: "Line style" }).getByRole("button", { name: "Curved lines" }).click();
  });

  test("layered timeline shows tracks per service and zooms", async ({ page }) => {
    const timeline = page.getByLabel("Timeline");
    const tracks = timeline.getByRole("list", { name: "Tracks" });
    for (const name of ["Actions", "Events", "Errors", "Requests", "Latency p95", "gateway", "orders"]) {
      await expect(tracks.getByText(name, { exact: true })).toBeVisible();
    }
    await timeline.getByRole("button", { name: "Pause live" }).click();
    const whole = timeline.getByRole("button", { name: "Whole range" });
    await expect(whole).toHaveAttribute("aria-pressed", "true");
    await timeline.getByRole("button", { name: "Zoom in time" }).click();
    await expect(whole).toHaveAttribute("aria-pressed", "false");
    await whole.click();
    await expect(whole).toHaveAttribute("aria-pressed", "true");
  });

  test("timeline cursor is dragged along the ruler and the dock is resizable", async ({ page }) => {
    const timeline = page.getByLabel("Timeline");
    await timeline.getByRole("button", { name: "Pause live" }).click();
    const ruler = timeline.getByLabel("Selected time");
    const box = await ruler.boundingBox();
    expect(box).not.toBeNull();
    if (!box) return;
    const before = await ruler.getAttribute("aria-valuetext");
    await page.mouse.move(box.x + box.width * 0.8, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width * 0.3, box.y + box.height / 2, { steps: 8 });
    await page.mouse.up();
    await expect(ruler).not.toHaveAttribute("aria-valuetext", before ?? "");
    await expect(timeline.getByText("paused", { exact: true })).toBeVisible();

    const dragged = await ruler.getAttribute("aria-valuetext");
    await ruler.focus();
    await page.keyboard.press("ArrowLeft");
    await expect(ruler).not.toHaveAttribute("aria-valuetext", dragged ?? "");

    const dock = page.getByRole("separator", { name: "Resize bottom dock" });
    const height = Number(await dock.getAttribute("aria-valuenow"));
    await dock.focus();
    await page.keyboard.press("ArrowUp");
    await expect(dock).toHaveAttribute("aria-valuenow", String(height + 24));
  });

  test("services can be switched off on the timeline, all at once or all but one", async ({ page }) => {
    const tracks = page.getByLabel("Timeline").getByRole("list", { name: "Tracks" });
    const orders = tracks.getByRole("checkbox", { name: "Show orders" });
    const gateway = tracks.getByRole("checkbox", { name: "Show gateway" });
    await expect(orders).toBeChecked();
    await orders.uncheck();
    await expect(orders).not.toBeChecked();
    await expect(gateway).toBeChecked();

    await tracks.getByRole("button", { name: "All off" }).click();
    await expect(gateway).not.toBeChecked();
    await expect(tracks.getByRole("button", { name: "All off" })).toBeDisabled();
    await tracks.getByRole("button", { name: "All on" }).click();
    await expect(gateway).toBeChecked();
    await expect(orders).toBeChecked();

    await tracks.getByRole("listitem").filter({ hasText: "orders" }).hover();
    await tracks.getByRole("button", { name: "Show only orders" }).click();
    await expect(orders).toBeChecked();
    await expect(gateway).not.toBeChecked();
    await tracks.getByRole("button", { name: "All on" }).click();
  });

  test("a replay range is dragged on the tracks, loops on play and resets with its close button", async ({ page }) => {
    const timeline = page.getByLabel("Timeline");
    const rows = timeline.locator('[data-area="rows"]');
    const box = await rows.boundingBox();
    expect(box).not.toBeNull();
    if (!box) return;
    const y = box.y + 30;
    await page.mouse.move(box.x + box.width * 0.3, y);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width * 0.32, y, { steps: 10 });
    await page.mouse.up();

    const chip = timeline.getByLabel("Replay range", { exact: true });
    await expect(chip).toBeVisible();
    await expect(timeline.getByText("paused", { exact: true })).toBeVisible();
    await expect(timeline.getByRole("button", { name: "Whole range" })).toHaveAttribute("aria-pressed", "false");

    const ruler = timeline.getByLabel("Selected time");
    await timeline.getByLabel("Speed").selectOption("10");
    await timeline.getByRole("button", { name: "Play replay" }).click();
    await expect(timeline.getByText("replay", { exact: true })).toBeVisible();
    // 2% of the screen is a bit over 2% of the 15-minute live window, about 20 s: at 10x one pass
    // takes about 2 s, so staying inside the range while still replaying for 3.6 s means it wrapped.
    for (let i = 0; i < 12; i++) {
      await page.waitForTimeout(300);
      const at = Number(await ruler.getAttribute("aria-valuenow"));
      expect(at).toBeGreaterThanOrEqual(dataPercent(0.3, box.width) - 1);
      expect(at).toBeLessThanOrEqual(dataPercent(0.32, box.width) + 1);
    }
    await expect(timeline.getByText("replay", { exact: true })).toBeVisible();

    await timeline.getByRole("button", { name: "Clear replay range" }).click();
    await expect(chip).toHaveCount(0);
    await expect(timeline.getByRole("button", { name: "Whole range" })).toHaveAttribute("aria-pressed", "true");
    await timeline.getByRole("button", { name: "Pause", exact: true }).click();
    await timeline.getByLabel("Speed").selectOption("1");
  });

  test("a click on the tracks moves the cursor and the tooltip sits above the pointer", async ({ page }) => {
    const timeline = page.getByLabel("Timeline");
    await timeline.getByRole("button", { name: "Pause live" }).click();
    const ruler = timeline.getByLabel("Selected time");
    const rows = timeline.locator('[data-area="rows"]');
    const box = await rows.boundingBox();
    expect(box).not.toBeNull();
    if (!box) return;
    await page.mouse.click(box.x + box.width * 0.5, box.y + 60);
    const at = Number(await ruler.getAttribute("aria-valuenow"));
    expect(Math.abs(at - dataPercent(0.5, box.width))).toBeLessThanOrEqual(2);
    await expect(timeline.getByLabel("Replay range")).toHaveCount(0);

    // Requests row, right edge: the tooltip must stay on screen and above the pointer.
    const px = box.x + box.width - 3;
    const py = box.y + 70;
    await page.mouse.move(px, py);
    const tip = page.getByRole("tooltip");
    await expect(tip).toBeVisible();
    const t = await tip.boundingBox();
    const viewport = page.viewportSize();
    expect(t).not.toBeNull();
    if (!t || !viewport) return;
    expect(t.y + t.height).toBeLessThanOrEqual(py);
    expect(t.x + t.width).toBeLessThanOrEqual(viewport.width);
    expect(t.x).toBeGreaterThanOrEqual(0);
  });

  test("the FUTURE stretch sits right of now; moving the cursor into it goes back to live", async ({ page }) => {
    const timeline = page.getByLabel("Timeline");
    const ruler = timeline.getByRole("slider", { name: "Selected time" });
    const live = timeline.getByText("live", { exact: true });
    const paused = timeline.getByText("paused", { exact: true });
    await expect(page.getByText("Starting engine…")).toBeHidden();
    await expect(live).toBeVisible();
    await expect(ruler).toHaveAttribute("aria-valuemax", "100");
    await expect(ruler).toHaveAttribute("aria-valuenow", "100");
    await expect(timeline.getByText("FUTURE", { exact: true })).toBeVisible();

    // Leave live, then drag the cursor into the future: it snaps back to live and keeps playing.
    let box = await ruler.boundingBox();
    expect(box).not.toBeNull();
    if (!box) return;
    await page.mouse.click(box.x + box.width * 0.5, box.y + box.height / 2);
    await expect(paused).toBeVisible();
    box = await ruler.boundingBox();
    if (!box) return;
    const y = box.y + box.height / 2;
    await page.mouse.move(box.x + box.width * 0.5, y);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width - FUTURE_PX / 3, y, { steps: 8 });
    await page.mouse.up();
    await expect(live).toBeVisible();
    await expect(timeline.getByRole("button", { name: "Pause live" })).toBeVisible();
    await expect(ruler).toHaveAttribute("aria-valuenow", "100");

    // A plain click there does the same.
    await timeline.getByRole("button", { name: "Pause live" }).click();
    await expect(paused).toBeVisible();
    box = await ruler.boundingBox();
    if (!box) return;
    await page.mouse.click(box.x + box.width - 10, box.y + box.height / 2);
    await expect(live).toBeVisible();

    // And so does the keyboard: one step right of the end.
    await timeline.getByRole("button", { name: "Pause live" }).click();
    await ruler.focus();
    await page.keyboard.press("End");
    await expect(paused).toBeVisible();
    await page.keyboard.press("ArrowRight");
    await expect(live).toBeVisible();

    const rows = timeline.locator('[data-area="rows"]');
    const area = await rows.boundingBox();
    expect(area).not.toBeNull();
    if (!area) return;
    await page.mouse.move(area.x + area.width - 10, area.y + 70);
    await expect(page.getByRole("tooltip")).toContainText("go back to live");
  });

  // Regression: zoomed in, cursor after "now", Play crashed the timeline with "Too many re-renders".
  test("zoomed in, the future returns to live and Play from the end starts over", async ({ page }) => {
    const errors = collectErrors(page);
    const timeline = page.getByLabel("Timeline");
    const ruler = timeline.getByRole("slider", { name: "Selected time" });
    await expect(page.getByText("Starting engine…")).toBeHidden();
    await timeline.getByRole("button", { name: "Zoom in time" }).click();
    const box = await ruler.boundingBox();
    expect(box).not.toBeNull();
    if (!box) return;
    await page.mouse.click(box.x + box.width - 10, box.y + box.height / 2);
    await expect(timeline.getByText("live", { exact: true })).toBeVisible();

    await timeline.getByRole("button", { name: "Pause live" }).click();
    await ruler.focus();
    await page.keyboard.press("End");
    await timeline.getByRole("button", { name: "Play replay" }).click();
    await expect(timeline.getByText("replay", { exact: true })).toBeVisible();
    // Starts over at the first loaded item (see the next test), which is before "now".
    await expect.poll(async () => Number(await ruler.getAttribute("aria-valuenow"))).toBeLessThan(100);
    const started = await ruler.getAttribute("aria-valuetext");
    await expect(ruler).not.toHaveAttribute("aria-valuetext", started ?? "");
    await expect(timeline.getByText("replay", { exact: true })).toBeVisible();
    expect(errors.filter((e) => /re-render/i.test(e))).toEqual([]);
    await timeline.getByRole("button", { name: "Pause", exact: true }).click();
  });

  test("Play from the end starts over at the first recorded item, not at the empty range start", async ({ page }) => {
    const timeline = page.getByLabel("Timeline");
    const ruler = timeline.getByRole("slider", { name: "Selected time" });
    const clockMs = async () => {
      const m = /(\d+):(\d+):(\d+)\.(\d+)/.exec((await ruler.getAttribute("aria-valuetext")) ?? "");
      return m ? ((Number(m[1]) * 60 + Number(m[2])) * 60 + Number(m[3])) * 1000 + Number(m[4]) : Number.NaN;
    };
    await expect(page.getByText("Starting engine…")).toBeHidden();
    await timeline.getByRole("button", { name: "Pause live" }).click();
    await timeline.getByLabel("Speed").selectOption("0.001");

    await ruler.focus();
    await page.keyboard.press("Home");
    await expect(ruler).toHaveAttribute("aria-valuenow", "0");
    const start = await ruler.getAttribute("aria-valuetext");
    await timeline.getByRole("button", { name: "Next event" }).click();
    await expect(ruler).not.toHaveAttribute("aria-valuetext", start ?? "");
    const first = await clockMs();

    await ruler.focus();
    await page.keyboard.press("End");
    await expect(ruler).toHaveAttribute("aria-valuenow", "100");
    await timeline.getByRole("button", { name: "Play replay" }).click();
    await expect(timeline.getByText("replay", { exact: true })).toBeVisible();
    await timeline.getByRole("button", { name: "Pause", exact: true }).click();
    // 0.001x: well under a millisecond of recorded time passes between Play and Pause.
    await expect.poll(async () => Math.abs((await clockMs()) - first)).toBeLessThanOrEqual(2);
    await timeline.getByLabel("Speed").selectOption("1");
  });

  test("the paused badge is not the degraded (amber) colour", async ({ page }) => {
    const timeline = page.getByLabel("Timeline");
    await timeline.getByRole("button", { name: "Pause live" }).click();
    const badge = timeline.getByText("paused", { exact: true });
    await expect(badge).toBeVisible();
    const color = await badge.evaluate((el) => getComputedStyle(el).color);
    expect(color).not.toMatch(/245,\s*158,\s*11/);
  });

  test("selecting a component opens its details in the side panel", async ({ page }) => {
    await expect(page.getByRole("region", { name: "Signals" })).toBeVisible();
    await page.getByRole("checkbox", { name: "Table view" }).check();
    await page.getByRole("table").getByRole("button", { name: "gateway", exact: true }).click();
    const inspector = page.getByRole("complementary", { name: "Inspector" });
    await expect(inspector.getByRole("heading", { name: "gateway", exact: true })).toBeVisible();
    await page.getByRole("tab", { name: "Signals" }).click();
    await expect(page.getByRole("region", { name: "Signals" }).getByLabel("Log records")).toBeVisible();
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
