import { expect, test } from "@playwright/test";
import { login } from "./helpers";

const WORKSPACE = "/app/isolation/isolation-test";

test("the live stream remains available while paused and clicking the ruler moves the playhead", async ({ page }) => {
  await login(page, "isolation@karbird.local", WORKSPACE);
  const timeline = page.getByRole("region", { name: "Timeline" });
  const slider = timeline.getByRole("slider", { name: "Selected time" });
  await expect(page.getByLabel("Live transport")).toHaveText("Live: WebSocket");
  await timeline.getByRole("button", { name: "Pause live" }).click();

  const beforeClick = await slider.getAttribute("aria-valuetext");
  const ruler = await slider.boundingBox();
  expect(ruler).not.toBeNull();
  if (!ruler) return;
  await page.mouse.click(ruler.x + ruler.width * 0.4, ruler.y + ruler.height / 2);
  await expect(slider).not.toHaveAttribute("aria-valuetext", beforeClick ?? "");
  const pausedAt = await slider.getAttribute("aria-valuetext");

  await expect(page.getByLabel("Live transport")).toHaveText("Live: WebSocket");
  await expect(slider).toHaveAttribute("aria-valuetext", pausedAt ?? "");
});

test("the paused summary keeps loading the off-screen live horizon", async ({ page }) => {
  const summaryEnds: bigint[] = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (!url.pathname.endsWith("/timeline/summary")) return;
    const to = url.searchParams.get("to");
    if (to && /^\d+$/.test(to)) summaryEnds.push(BigInt(to));
  });

  await login(page, "isolation@karbird.local", WORKSPACE);
  const timeline = page.getByRole("region", { name: "Timeline" });
  const slider = timeline.getByRole("slider", { name: "Selected time" });
  await expect(page.getByLabel("Live transport")).toHaveText("Live: WebSocket");
  await timeline.getByRole("button", { name: "Pause live" }).click();
  await timeline.getByRole("button", { name: "Zoom out time" }).click();

  await expect.poll(() => summaryEnds.length).toBeGreaterThan(0);
  const viewedAt = await slider.getAttribute("aria-valuetext");
  const requestedThrough = summaryEnds.at(-1);
  expect(requestedThrough).toBeDefined();
  if (requestedThrough === undefined) return;

  await expect.poll(() => summaryEnds.some((to) => to > requestedThrough), { timeout: 20_000 }).toBe(true);
  await expect(slider).toHaveAttribute("aria-valuetext", viewedAt ?? "");
});

test("panning across a loaded summary does not request or cover the same data again", async ({ page }) => {
  const summaryRequests: string[] = [];
  const completedSummaries: string[] = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname.endsWith("/timeline/summary")) summaryRequests.push(`${url.searchParams.get("from")}:${url.searchParams.get("to")}`);
  });
  page.on("response", (response) => {
    const url = new URL(response.url());
    if (url.pathname.endsWith("/timeline/summary") && response.ok()) completedSummaries.push(`${url.searchParams.get("from")}:${url.searchParams.get("to")}`);
  });

  await login(page, "isolation@karbird.local", WORKSPACE);
  const timeline = page.getByRole("region", { name: "Timeline" });
  const slider = timeline.getByRole("slider", { name: "Selected time" });
  await timeline.getByRole("button", { name: "Pause live" }).click();
  await timeline.getByRole("button", { name: "Zoom out time" }).click();
  await expect.poll(() => summaryRequests.length).toBeGreaterThan(0);
  await expect.poll(() => {
    const latest = summaryRequests.at(-1);
    return latest !== undefined && completedSummaries.includes(latest);
  }).toBe(true);

  const loadedRequests = new Set(summaryRequests);
  const requestsBeforePan = summaryRequests.length;
  const rows = timeline.locator('[data-area="rows"]');
  const bounds = await rows.boundingBox();
  expect(bounds).not.toBeNull();
  if (!bounds) return;
  const ruler = slider.locator("canvas");
  const beforePan = await ruler.screenshot();
  const y = bounds.y + bounds.height / 2;
  const x = bounds.x + bounds.width * 0.65;
  await page.keyboard.down("Shift");
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + bounds.width * 0.15, y);
  await page.mouse.up();
  await page.keyboard.up("Shift");

  const afterPan = await ruler.screenshot();
  expect(afterPan.equals(beforePan)).toBe(false);
  await expect.poll(() => summaryRequests.slice(requestsBeforePan).every((range) => completedSummaries.includes(range))).toBe(true);
  const addedRequests = summaryRequests.slice(requestsBeforePan);
  expect(addedRequests.every((range) => !loadedRequests.has(range))).toBe(true);
  await expect(timeline.getByRole("img", { name: /Loading timeline data/ })).toHaveCount(0);
});
