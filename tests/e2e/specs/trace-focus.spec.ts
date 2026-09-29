import { expect, test, type Page } from "@playwright/test";
import { collectErrors, login } from "./helpers";

const WORKSPACE = "/app/demo/local-demo";

function serviceRows(page: Page) {
  return page.getByLabel("Timeline").getByRole("list", { name: "Tracks" }).locator("[data-service-color]");
}

/** "shown / in window" from the engine diagnostics popover. */
async function hopsInFrame(page: Page): Promise<[number, number]> {
  const text = (await page.getByText("Hops in frame").locator("..").textContent()) ?? "";
  const m = text.match(/(\d+)\s*\/\s*(\d+)/);
  return [Number(m?.[1] ?? -1), Number(m?.[2] ?? -1)];
}

test.describe("service colours and request focus on demo data", () => {
  test.beforeEach(async ({ page }) => {
    await login(page, "admin@controlhub.local", WORKSPACE);
    await expect(page.getByText("Starting engine…")).toBeHidden();
  });

  test("every service has its own colour; the legend tells request and response dots apart", async ({ page }) => {
    const errors = collectErrors(page);
    const rows = serviceRows(page);
    await expect.poll(() => rows.count()).toBeGreaterThanOrEqual(5);
    const colors = await rows.evaluateAll((els) => els.map((e) => e.getAttribute("data-service-color") ?? ""));
    expect(colors.every((c) => /^#[0-9a-f]{6}$/.test(c))).toBe(true);
    expect(new Set(colors).size).toBe(colors.length);

    await expect(page.getByText("Request", { exact: true })).toBeVisible();
    await expect(page.getByText("Response", { exact: true })).toBeVisible();
    const note = page.getByLabel("Request animation");
    await expect(note).toContainText("filled dot = request, ring = response");
    await expect(note).toContainText("dot colour = service where the request started");
    expect(errors).toEqual([]);
  });

  test("selecting a request shows only it on the map and timeline and replays it", async ({ page }) => {
    const errors = collectErrors(page);
    const panel = page.getByRole("region", { name: "Signals" });
    await panel.getByRole("tab", { name: "Traces" }).click();
    const list = panel.getByRole("list", { name: "Traces" });
    // A request that crosses several services.
    const big = list.getByRole("button").filter({ hasText: /\D([4-9]|\d{2,})sp$/ }).first();
    await expect(big).toBeVisible();
    const spanCount = Number((await big.textContent())?.match(/(\d+)sp$/)?.[1] ?? 0);
    await big.click();

    const focus = page.getByRole("region", { name: "Focused request" });
    await expect(focus).toBeVisible();
    const only = focus.getByRole("checkbox", { name: "Only this request" });
    await expect(only).toBeChecked();
    const size = focus.getByLabel("Request size");
    await expect(size).toContainText(/\d+ spans · \d+ services/);
    const services = Number((await size.textContent())?.match(/(\d+) services/)?.[1] ?? 0);
    expect(services).toBeGreaterThan(1);

    // Replayed on a loop over the request's own time span.
    const timeline = page.getByLabel("Timeline");
    await expect(timeline.getByText("replay", { exact: true })).toBeVisible();

    // The timeline keeps only the services this request went through.
    const rows = serviceRows(page);
    await expect.poll(() => rows.count()).toBe(services);

    // The map animates only this request's calls.
    await page.getByRole("button", { name: /^Engine/ }).click();
    await expect
      .poll(async () => {
        const [, inWindow] = await hopsInFrame(page);
        return inWindow >= 0 && inWindow <= spanCount;
      })
      .toBe(true);

    // Everything else comes back when isolation is switched off.
    await only.uncheck();
    await expect.poll(() => rows.count()).toBeGreaterThan(services);
    await only.check();
    await expect.poll(() => rows.count()).toBe(services);

    // Closing the focus clears the selection and the replay range.
    await focus.getByRole("button", { name: "Stop focusing this request" }).click();
    await expect(focus).toBeHidden();
    await expect(timeline.getByText("paused", { exact: true })).toBeVisible();
    await expect(list.locator('button[aria-current="true"]')).toHaveCount(0);
    await expect.poll(() => rows.count()).toBeGreaterThan(services);
    expect(errors).toEqual([]);
  });
});
