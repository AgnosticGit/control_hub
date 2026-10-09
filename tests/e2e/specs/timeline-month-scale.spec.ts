import { expect, test } from "@playwright/test";
import { collectErrors, login } from "./helpers";

const WORKSPACE = "/app/isolation/isolation-test";
const DAY_NS = 86_400_000_000_000n;

test("month-scale timeline loads bounded summaries and pages through older history", async ({ page }) => {
  test.setTimeout(90_000);
  const errors = collectErrors(page);
  await login(page, "isolation@karbird.local", WORKSPACE);
  await page.emulateMedia({ reducedMotion: "reduce" });
  const timeline = page.getByLabel("Timeline");
  await expect(timeline.getByRole("slider", { name: "Selected time" })).toBeVisible();
  const pauseLive = timeline.getByRole("button", { name: "Pause live" });
  if (await pauseLive.count()) await pauseLive.click();

  let releaseHeldResponse!: () => void;
  const heldResponse = new Promise<void>((resolve) => { releaseHeldResponse = resolve; });
  let held = false;
  await page.route("**/timeline/summary**", async (route) => {
    const url = new URL(route.request().url());
    const from = BigInt(url.searchParams.get("from") ?? "0");
    const to = BigInt(url.searchParams.get("to") ?? "0");
    if (held || to - from < 29n * DAY_NS) return route.continue();
    held = true;
    const response = await route.fetch();
    await heldResponse;
    await route.fulfill({ response });
  });
  const summaryForMonth = page.waitForResponse((response) => {
    const url = new URL(response.url());
    if (!url.pathname.endsWith("/timeline/summary") || response.status() !== 200) return false;
    const from = BigInt(url.searchParams.get("from") ?? "0");
    const to = BigInt(url.searchParams.get("to") ?? "0");
    return to - from === 30n * DAY_NS;
  });
  const zoomOut = timeline.getByRole("button", { name: "Zoom out time" });
  for (let i = 0; i < 12; i++) await zoomOut.click();
  await expect(timeline.getByRole("status")).toHaveText(/Loading (earlier|later) timeline data/);
  await expect(timeline.getByRole("slider", { name: "Selected time" })).toBeVisible();
  releaseHeldResponse();
  await summaryForMonth;
  await expect(timeline.getByText("30 days", { exact: true })).toBeVisible();

  const olderSummary = page.waitForResponse((response) => {
    const url = new URL(response.url());
    return url.pathname.endsWith("/timeline/summary") && response.status() === 200;
  });
  const rows = timeline.locator('[data-area="rows"]');
  await rows.hover();
  await page.keyboard.down("Shift");
  await page.mouse.wheel(-420, 0);
  await page.keyboard.up("Shift");
  await olderSummary;
  expect(errors).toEqual([]);
});
