import { expect, test, type Locator } from "@playwright/test";
import { login } from "./helpers";

const WORKSPACE = "/app/demo/local-demo";
const LIVE_WINDOW_MS = 15 * 60 * 1000;

async function clockMs(timeline: Locator) {
  const value = await timeline.getByRole("slider", { name: "Selected time" }).getAttribute("aria-valuetext");
  const parsed = value ? Date.parse(value) : Number.NaN;
  if (!Number.isFinite(parsed)) throw new Error(`could not parse live timeline time: ${value}`);
  return parsed;
}

async function requestBarHeight(canvas: Locator, nowMs: number, targetMs: number | null) {
  return canvas.evaluate((element, args) => {
    const canvas = element as HTMLCanvasElement;
    const context = canvas.getContext("2d");
    if (!context) return null;
    const rect = canvas.getBoundingClientRect();
    const scale = canvas.width / rect.width;
    const width = rect.width;
    const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
    const rowTop = Math.round(58 * scale);
    const rowBottom = Math.round(82 * scale);
    const bluePixelsAt = (x: number) => {
      let count = 0;
      for (let y = rowTop; y < rowBottom; y++) {
        const offset = (y * canvas.width + x) * 4;
        if ((pixels[offset] ?? 0) === 59 && (pixels[offset + 1] ?? 0) === 130 && (pixels[offset + 2] ?? 0) === 246 && (pixels[offset + 3] ?? 0) > 200) count++;
      }
      return count;
    };
    const dataFromMs = args.nowMs - args.liveWindowMs;
    const dataSpanMs = args.liveWindowMs;
    let target = args.targetMs;
    if (target === null) {
      let runStart = -1;
      let best: { x: number; targetMs: number } | null = null;
      for (let x = Math.ceil(20 * scale); x < (width - 240) * scale; x++) {
        const filled = bluePixelsAt(x) > 0;
        if (filled && runStart < 0) runStart = x;
        if ((!filled || x === Math.floor(width * scale) - 1) && runStart >= 0) {
          const end = filled ? x : x - 1;
          const center = Math.floor((runStart + end) / 2);
          const cssX = center / scale;
          const bucketTime = dataFromMs + ((cssX + 1) / width) * dataSpanMs;
          if (end - runStart >= Math.ceil(2 * scale) && cssX < width - 240) best = { x: center, targetMs: bucketTime };
          runStart = -1;
        }
      }
      if (!best) return null;
      target = best.targetMs;
    }
    const cssX = ((target - dataFromMs) / dataSpanMs) * width;
    const center = Math.round(cssX * scale);
    let height = 0;
    for (let x = Math.max(0, center - 1); x <= Math.min(canvas.width - 1, center + 1); x++) height = Math.max(height, bluePixelsAt(x));
    return height > 0 ? { targetMs: target, height } : null;
  }, { nowMs, targetMs, liveWindowMs: LIVE_WINDOW_MS });
}

test("live request bars keep the same height as the timeline advances", async ({ page }) => {
  await login(page, "admin@karbird.local", WORKSPACE);
  const timeline = page.getByLabel("Timeline");
  await expect(timeline.getByText("Requests", { exact: true })).toBeVisible();
  await timeline.getByRole("button", { name: "Zoom in time" }).click();
  await timeline.getByRole("button", { name: "Zoom in time" }).click();
  await expect(timeline.getByTitle("Visible time span")).toHaveText("15 min");
  const canvas = timeline.locator("canvas").nth(1);
  const slider = timeline.getByRole("slider", { name: "Selected time" });
  const beforeTime = await slider.getAttribute("aria-valuetext");
  const initialResult: { value: Awaited<ReturnType<typeof requestBarHeight>> } = { value: null };
  await expect.poll(async () => {
    initialResult.value = await requestBarHeight(canvas, await clockMs(timeline), null);
    return initialResult.value?.height ?? 0;
  }, { timeout: 20_000 }).toBeGreaterThanOrEqual(16);
  const initial = initialResult.value;
  if (!initial) throw new Error("no stable request bucket was visible on the timeline");

  await expect.poll(() => slider.getAttribute("aria-valuetext"), { timeout: 10_000 }).not.toBe(beforeTime);
  await expect.poll(async () => requestBarHeight(canvas, await clockMs(timeline), initial?.targetMs ?? null), { timeout: 10_000 })
    .toEqual({ targetMs: initial.targetMs, height: initial.height });
});
