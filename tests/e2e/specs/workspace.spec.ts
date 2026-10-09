import { expect, test } from "@playwright/test";
import { mapLayer, collectErrors, login } from "./helpers";

const WORKSPACE = "/app/demo/local-demo";
/** Slider value (0 = start of the loaded data, 100 = now) at a share of the timeline. */
const dataPercent = (share: number) => share * 100;

test.describe("workspace on demo data", () => {
  test.beforeEach(async ({ page }) => {
    await login(page, "admin@karbird.local", WORKSPACE);
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
    await (await mapLayer(page, "Table view")).check();
    const table = page.getByRole("table");
    await expect(table.getByRole("button", { name: "gateway", exact: true })).toBeVisible();
    await expect(table.getByRole("button", { name: "orders", exact: true })).toBeVisible();

    await table.getByRole("button", { name: "orders", exact: true }).click();
    const inspector = page.getByRole("complementary", { name: "Inspector" });
    await expect(inspector.getByRole("heading", { name: "orders", exact: true })).toBeVisible();
    await expect(inspector.getByText(/observed|inferred/).first()).toBeVisible();
  });

  test("scope menus match the site style and can be operated from the keyboard", async ({ page }) => {
    for (const label of ["Organization", "Project", "Environment"]) {
      const trigger = page.getByRole("combobox", { name: label });
      await expect(trigger).toBeVisible();
      const triggerStyle = await trigger.evaluate((element) => {
        const style = getComputedStyle(element);
        return { height: style.height, borderRadius: style.borderRadius, backgroundColor: style.backgroundColor };
      });
      expect(triggerStyle.height).toBe("36px");
      expect(triggerStyle.borderRadius).toBe("8px");

      await trigger.click();
      const menu = page.getByRole("listbox", { name: `${label} options` });
      await expect(menu).toBeVisible();
      const menuStyle = await menu.evaluate((element) => {
        const style = getComputedStyle(element);
        return { borderRadius: style.borderRadius, backgroundColor: style.backgroundColor, borderStyle: style.borderStyle, width: element.getBoundingClientRect().width };
      });
      expect(menuStyle.borderRadius).toBe("12px");
      expect(menuStyle.backgroundColor).not.toBe("rgba(0, 0, 0, 0)");
      expect(menuStyle.borderStyle).toBe("solid");
      expect(menuStyle.width).toBeCloseTo(await trigger.evaluate((element) => element.getBoundingClientRect().width), 0);
      await trigger.press("ArrowDown");
      await trigger.press("Enter");
      await expect(menu).toBeHidden();
    }
  });

  test("the workspace has no top header and the sidebar can be hidden and restored", async ({ page }) => {
    await expect(page.getByRole("group", { name: "Load a recent range" })).toHaveCount(0);
    await expect(page.getByRole("banner")).toHaveCount(0);
    await page.getByRole("button", { name: "Hide sidebar" }).click();
    await expect(page.locator('[aria-label="Sidebar"]')).toHaveAttribute("aria-hidden", "true");
    await expect(page.getByRole("button", { name: "Show sidebar" })).toBeVisible();

    await page.reload();
    await expect(page.getByRole("button", { name: "Show sidebar" })).toBeVisible();
    await expect(page.locator('[aria-label="Sidebar"]')).toHaveAttribute("aria-hidden", "true");
    await page.getByRole("button", { name: "Show sidebar" }).click();
    await expect(page.getByRole("navigation", { name: "Workspace" })).toBeVisible();
  });

  test("System Map can enter and exit full screen", async ({ page }) => {
    const systemMap = page.getByRole("main", { name: "System map" });
    await page.getByRole("button", { name: "Full screen" }).click();
    await expect.poll(() => systemMap.evaluate((element) => element.matches(":fullscreen"))).toBe(true);
    await expect(page.getByRole("button", { name: "Exit full screen" })).toBeVisible();

    await page.keyboard.press("Escape");
    await expect.poll(() => systemMap.evaluate((element) => element.matches(":fullscreen"))).toBe(false);
    await expect(page.getByRole("button", { name: "Full screen" })).toBeVisible();
  });

  test("selecting a service highlights all its incoming and outgoing lines", async ({ page }) => {
    const tableView = await mapLayer(page, "Table view");
    await tableView.check();
    await page.getByRole("table").getByRole("button", { name: "gateway", exact: true }).click();
    await expect(page.getByRole("complementary", { name: "Inspector" }).getByRole("heading", { name: "gateway", exact: true })).toBeVisible();

    await (await mapLayer(page, "Table view")).uncheck();
    const incidentLines = page.locator("[data-selected-incident-edges]");
    await expect.poll(async () => (await incidentLines.getAttribute("data-selected-incident-edges"))?.split(",").filter(Boolean).length ?? 0)
      .toBeGreaterThan(1);
  });

  test("zooming out stops at a fitted overview instead of losing the map", async ({ page }) => {
    const map = page.getByRole("application", { name: /Service map/ });
    await expect(map).toHaveAttribute("data-drawn-nodes", /\d/);
    await map.focus();
    for (let i = 0; i < 40; i++) await page.keyboard.press("-");
    const camera = async () => JSON.parse((await map.getAttribute("data-camera")) ?? "null") as { x: number; y: number; scale: number } | null;
    await expect.poll(async () => (await camera())?.scale ?? 4).toBeLessThan(1);
    const atLimit = await camera();
    expect(atLimit).not.toBeNull();

    for (let i = 0; i < 8; i++) await page.keyboard.press("-");
    await expect.poll(camera).toEqual(atLimit);
  });

  test("map renders on a canvas when WebGL is available", async ({ page }) => {
    const canvas = page.locator("main canvas");
    const fallback = page.getByText(/WebGL is unavailable/);
    await expect(canvas.or(fallback)).toBeVisible();
    await page.screenshot({ path: "../../test-results/e2e/workspace.png" });
  });

  test("the map has a faint grid that moves with the map when it is dragged", async ({ page }) => {
    const canvas = page.locator("main canvas");
    await expect(canvas.or(page.getByText(/WebGL is unavailable/))).toBeVisible();
    test.skip((await page.getByText(/WebGL is unavailable/).count()) > 0, "no WebGL in this browser");
    const errors = collectErrors(page);

    // Columns along a row near the top edge (no cards there) that are a little brighter than the rest.
    const gridColumns = async () => {
      const png = (await canvas.screenshot()).toString("base64");
      return page.evaluate(async (b64) => {
        const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
        const bmp = await createImageBitmap(new Blob([bytes], { type: "image/png" }));
        const c = new OffscreenCanvas(bmp.width, bmp.height);
        const ctx = c.getContext("2d");
        if (!ctx) return { cols: [] as number[], peak: 0 };
        ctx.drawImage(bmp, 0, 0);
        const width = Math.floor(bmp.width * 0.6);
        const row = ctx.getImageData(0, 6, width, 1).data;
        const lum: number[] = [];
        for (let x = 0; x < width; x++) lum.push((row[x * 4]! + row[x * 4 + 1]! + row[x * 4 + 2]!) / 3);
        const base = [...lum].sort((a, b) => a - b)[Math.floor(lum.length / 2)]!;
        const cols = lum.flatMap((v, x) => (v - base >= 2 ? [x] : []));
        return { cols, peak: Math.max(...lum) - base };
      }, png);
    };

    const before = await gridColumns();
    expect(before.cols.length).toBeGreaterThanOrEqual(5);
    // Very pale: lines are barely brighter than the background.
    expect(before.peak).toBeLessThan(40);
    const gaps = before.cols.slice(1).map((x, i) => x - before.cols[i]!).filter((g) => g > 2);
    expect(Math.min(...gaps)).toBeGreaterThanOrEqual(14);

    const box = await canvas.boundingBox();
    if (!box) throw new Error("map canvas has no box");
    await page.mouse.move(box.x + box.width * 0.3, box.y + box.height * 0.5);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width * 0.3 + 7, box.y + box.height * 0.5, { steps: 4 });
    await page.mouse.up();
    await expect.poll(async () => (await gridColumns()).cols.join(",")).not.toBe(before.cols.join(","));
    expect(errors).toEqual([]);
  });

  test("cards panned off-screen stay drawn, so panning back never draws them again", async ({ page }) => {
    const canvas = page.locator("main canvas");
    await expect(canvas.or(page.getByText(/WebGL is unavailable/))).toBeVisible();
    test.skip((await page.getByText(/WebGL is unavailable/).count()) > 0, "no WebGL in this browser");
    const map = page.getByRole("application", { name: /Service map/ });
    await expect(map).toHaveAttribute("data-drawn-nodes", /\d/);
    // The drawn cards after `n` more engine frames (each frame rewrites the attribute).
    const drawnAfterFrames = async (n = 2) =>
      new Set(
        (
          await map.evaluate(
            (el, count) =>
              new Promise<string>((resolve) => {
                let seen = 0;
                const mo = new MutationObserver(() => {
                  if (++seen < count) return;
                  mo.disconnect();
                  resolve(el.getAttribute("data-drawn-nodes") ?? "");
                });
                mo.observe(el, { attributes: true, attributeFilter: ["data-drawn-nodes"] });
              }),
            n,
          )
        )
          .split(",")
          .filter(Boolean),
      );
    const missing = (all: Set<string>, now: Set<string>) => [...all].filter((i) => !now.has(i));

    const systemMap = page.getByRole("main", { name: "System map" });
    await systemMap.getByRole("button", { name: "Fit to screen" }).click();
    const all = await drawnAfterFrames();
    expect(all.size).toBeGreaterThan(3);
    for (let i = 0; i < 3; i++) await systemMap.getByRole("button", { name: "Zoom in" }).click();
    // Zoomed in, cards beyond the screen edge are still drawn.
    expect(missing(all, await drawnAfterFrames())).toEqual([]);

    const box = await canvas.boundingBox();
    if (!box) throw new Error("map canvas has no box");
    const y = box.y + box.height * 0.5;
    await page.mouse.move(box.x + box.width * 0.35, y);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width * 0.65, y, { steps: 6 });
    // While the button is still held: nothing that was drawn disappears.
    expect(missing(all, await drawnAfterFrames())).toEqual([]);
    await page.mouse.up();
    expect(missing(all, await drawnAfterFrames())).toEqual([]);
    // And back again.
    await page.mouse.move(box.x + box.width * 0.65, y);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width * 0.35, y, { steps: 6 });
    expect(missing(all, await drawnAfterFrames())).toEqual([]);
    await page.mouse.up();
  });

  test("zooming the map all the way out keeps every card's name, just smaller", async ({ page }) => {
    const canvas = page.locator("main canvas");
    await expect(canvas.or(page.getByText(/WebGL is unavailable/))).toBeVisible();
    test.skip((await page.getByText(/WebGL is unavailable/).count()) > 0, "no WebGL in this browser");
    const map = page.getByRole("application", { name: /Service map/ });
    await expect(map).toHaveAttribute("data-drawn-nodes", /\d/);
    const systemMap = page.getByRole("main", { name: "System map" });
    await systemMap.getByRole("button", { name: "Fit to screen" }).click();
    // Far past the minimum zoom; the old level of detail hid all names below 0.45×.
    for (let i = 0; i < 15; i++) await systemMap.getByRole("button", { name: "Zoom out" }).click();
    const ids = (attr: string | null) => (attr ?? "").split(",").filter(Boolean).sort().join(",");
    await expect
      .poll(async () => {
        const [drawn, labelled] = await Promise.all([map.getAttribute("data-drawn-nodes"), map.getAttribute("data-labelled-nodes")]);
        return ids(drawn) !== "" && ids(drawn) === ids(labelled);
      })
      .toBe(true);
  });

  test("returning to the overview from other pages shows it at once with its state kept", async ({ page }) => {
    const timeline = page.getByLabel("Timeline");
    await expect(page.getByText("Starting engine…")).toBeHidden();
    await timeline.getByRole("button", { name: "Pause live" }).click();
    await expect(timeline.getByText("paused", { exact: true })).toBeVisible();
    await (await mapLayer(page, "Table view")).check();
    await expect(page.getByRole("table")).toBeVisible();

    let workers = 0;
    page.on("worker", () => workers++);
    const sidebar = page.getByRole("navigation", { name: "Workspace" });

    await sidebar.getByRole("link", { name: "Integration" }).click();
    await expect(page).toHaveURL(/\/integration$/);
    await expect(page.getByRole("main", { name: "System map" })).toHaveCount(0);
    await page.getByRole("navigation", { name: "Workspace" }).getByRole("link", { name: "Overview" }).click();
    await expect(page).toHaveURL(new RegExp(`${WORKSPACE}$`));
    await expect(await mapLayer(page, "Table view")).toBeChecked();
    await expect(page.getByText("Starting engine…")).toHaveCount(0);
    await expect(page.getByLabel("Timeline").getByText("paused", { exact: true })).toBeVisible();

    // A page outside the project (the account) keeps it too.
    await page.getByRole("link", { name: "Your account" }).click();
    await expect(page).toHaveURL(/\/app\/account/);
    await page.goBack();
    await expect(page).toHaveURL(new RegExp(`${WORKSPACE}$`));
    await expect(await mapLayer(page, "Table view")).toBeChecked();
    await expect(page.getByText("Starting engine…")).toHaveCount(0);
    await expect(page.getByLabel("Timeline").getByText("paused", { exact: true })).toBeVisible();

    // The engine worker was never started again.
    expect(workers).toBe(0);
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
    await page.mouse.move(box.x + box.width * 0.305, y, { steps: 10 });
    await page.mouse.up();

    const chip = timeline.getByLabel("Replay range", { exact: true });
    await expect(chip).toBeVisible();
    await expect(timeline.getByText("paused", { exact: true })).toBeVisible();
    await expect(timeline.getByRole("button", { name: "Whole range" })).toHaveAttribute("aria-pressed", "false");

    const ruler = timeline.getByLabel("Selected time");
    await timeline.getByRole("button", { name: "Play replay" }).click();
    await expect(timeline.getByText("replay", { exact: true })).toBeVisible();
    // A short range should loop at the fixed readable replay pace.
    for (let i = 0; i < 20; i++) {
      await page.waitForTimeout(300);
      const at = Number(await ruler.getAttribute("aria-valuenow"));
      expect(at).toBeGreaterThanOrEqual(dataPercent(0.3) - 1);
      expect(at).toBeLessThanOrEqual(dataPercent(0.305) + 1);
    }
    await expect(timeline.getByText("replay", { exact: true })).toBeVisible();

    await timeline.getByRole("button", { name: "Clear replay range" }).click();
    await expect(chip).toHaveCount(0);
    await expect(timeline.getByRole("button", { name: "Whole range" })).toHaveAttribute("aria-pressed", "true");
    await timeline.getByRole("button", { name: "Pause", exact: true }).click();
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
    expect(Math.abs(at - dataPercent(0.5))).toBeLessThanOrEqual(2);
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

  test("dragging the playhead from the middle of a track scrubs instead of selecting a range", async ({ page }) => {
    const timeline = page.getByRole("region", { name: "Timeline" });
    await timeline.getByRole("button", { name: "Pause live" }).click();
    const ruler = timeline.getByRole("slider", { name: "Selected time" });
    const rows = timeline.locator('[data-area="rows"]');
    const rulerBox = await ruler.boundingBox();
    const rowsBox = await rows.boundingBox();
    expect(rulerBox).not.toBeNull();
    expect(rowsBox).not.toBeNull();
    if (!rulerBox || !rowsBox) return;
    const before = await ruler.getAttribute("aria-valuenow");
    const x = rulerBox.x + rulerBox.width - 72;
    const y = rowsBox.y + rowsBox.height / 2;
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.mouse.move(x - 48, y, { steps: 5 });
    await page.mouse.up();
    await expect(ruler).not.toHaveAttribute("aria-valuenow", before ?? "");
    await expect(timeline.getByRole("button", { name: "Clear replay range" })).toHaveCount(0);
  });

  test("the timeline can grow to its rows but does not retain blank height", async ({ page }) => {
    const timeline = page.getByRole("region", { name: "Timeline" });
    const splitter = page.getByRole("separator", { name: "Resize bottom dock" });
    await expect(timeline.getByRole("list", { name: "Tracks" })).toBeVisible();
    const max = Number(await splitter.getAttribute("aria-valuemax"));
    expect(max).toBeLessThan(640);
    await splitter.focus();
    await page.keyboard.press("End");
    const tracks = timeline.getByRole("list", { name: "Tracks" });
    const sizes = await tracks.evaluate((element) => {
      const scroller = element.parentElement?.parentElement;
      return { scrollHeight: scroller?.scrollHeight ?? 0, clientHeight: scroller?.clientHeight ?? 0 };
    });
    expect(sizes.scrollHeight).toBeLessThanOrEqual(sizes.clientHeight + 1);
  });

  test("the timeline has no future section and can return to live", async ({ page }) => {
    const timeline = page.getByLabel("Timeline");
    const ruler = timeline.getByRole("slider", { name: "Selected time" });
    const live = timeline.getByText("live", { exact: true });
    const paused = timeline.getByText("paused", { exact: true });
    await expect(page.getByText("Starting engine…")).toBeHidden();
    await expect(live).toBeVisible();
    await expect(ruler).toHaveAttribute("aria-valuemax", "100");
    await expect(ruler).toHaveAttribute("aria-valuenow", "100");
    await expect(timeline.getByText("FUTURE", { exact: true })).toHaveCount(0);

    // Move inside the loaded time range, then return explicitly to the current live point.
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
    await page.mouse.move(box.x + box.width * 0.75, y, { steps: 8 });
    await page.mouse.up();
    await expect(paused).toBeVisible();
    await timeline.getByRole("button", { name: "Return to live" }).click();
    await expect(live).toBeVisible();
    await expect(ruler).toHaveAttribute("aria-valuenow", "100");

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
  test("zoomed in, moving beyond the end returns to live and Play from the end starts over", async ({ page }) => {
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
    // Playback restarts from the first recorded item at its default readable pace.
    await expect.poll(async () => Math.abs((await clockMs()) - first)).toBeLessThanOrEqual(5_000);
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
    await (await mapLayer(page, "Table view")).check();
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
  await login(page, "isolation@karbird.local", WORKSPACE);
  await expect(page.getByText(/does not exist or you do not have access/)).toBeVisible();
});
