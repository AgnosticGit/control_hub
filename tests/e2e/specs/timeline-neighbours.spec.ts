import { expect, test, type Page } from "./isolated-test";
import { collectErrors, ingestSpans, login, randomHex } from "./helpers";

const WORKSPACE = "/app/isolation/isolation-test";
const MS = 1_000_000n;

/** Cursor time of the timeline ruler in ms since midnight, from its aria-valuetext. */
async function cursorMs(page: Page): Promise<number> {
  const text = (await page.getByLabel("Timeline").getByRole("slider", { name: "Selected time" }).getAttribute("aria-valuetext")) ?? "";
  const m = /(\d+):(\d+):(\d+)/.exec(text);
  return m ? ((Number(m[1]) * 60 + Number(m[2])) * 60 + Number(m[3])) * 1000 : Number.NaN;
}

/** Pauses, puts the cursor in the quiet gap between the two events and zooms in tightly. */
async function zoomIntoQuietStretch(page: Page) {
  const timeline = page.getByLabel("Timeline");
  await expect(page.getByText("Starting engine…")).toBeHidden();
  await timeline.getByRole("button", { name: "Pause live" }).click();
  const ruler = timeline.getByRole("slider", { name: "Selected time" });
  // The live ruler shows one hour. Near its right edge is about 36 seconds ago,
  // between the fixture events at two minutes and five seconds ago.
  const liveTime = await ruler.getAttribute("aria-valuetext");
  await ruler.focus();
  await page.keyboard.press("End");
  await page.keyboard.press("ArrowLeft");
  await expect(ruler).not.toHaveAttribute("aria-valuetext", liveTime ?? "");
  const zoomIn = timeline.getByRole("button", { name: "Zoom in time" });
  await zoomIn.focus();
  for (let i = 0; i < 12; i++) await zoomIn.click();
  await expect(timeline.getByRole("button", { name: "Whole range" })).toHaveAttribute("aria-pressed", "false");
}

test("a zoomed-in stretch with nothing in it offers the nearest item on each side", async ({ page }) => {
  test.setTimeout(90_000);
  const errors = collectErrors(page);
  const service = `quiet-${randomHex(4)}`;
  const now = BigInt(Date.now()) * MS;
  const minute = 60_000n * MS;
  const span = (at: bigint) => ({ traceId: randomHex(16), spanId: randomHex(8), name: "GET /quiet", kind: 2, startNs: at, endNs: at + 30n * MS });
  // One request on either side of the stretch near the live edge.
  await ingestSpans(service, [span(now - 2n * minute), span(now - 5n * 1_000n * MS)]);

  await login(page, "isolation@karbird.local", WORKSPACE);
  const timeline = page.getByLabel("Timeline");
  const earlier = timeline.getByRole("button", { name: /^Go to the nearest earlier item/ });
  const later = timeline.getByRole("button", { name: /^Go to the nearest later item/ });

  // Wait for the backend to store the spans before the page reads its service list.
  const only = timeline.getByRole("button", { name: `Show only ${service}`, exact: true });
  const from = (now - 10n * minute).toString();
  const to = (now + minute).toString();
  await expect.poll(async () => {
    const response = await page.request.get(`/api/v1/projects/isolation/environments/isolation-test/services?from=${from}&to=${to}`);
    if (!response.ok()) return false;
    const result = await response.json() as { services: Array<{ name: string }> };
    return result.services.some((item) => item.name === service);
  }, { timeout: 60_000 }).toBe(true);
  await page.reload();
  await expect(page.getByText("Starting engine…")).toBeHidden();
  await expect(only).toBeVisible({ timeout: 30_000 });
  await only.click();
  await zoomIntoQuietStretch(page);
  await expect(earlier).toBeVisible({ timeout: 30_000 });
  await expect(later).toBeVisible({ timeout: 30_000 });

  await expect(timeline.getByText("Nothing in this stretch of time")).toBeVisible();
  await expect(earlier).toContainText(/\d/);
  await expect(later).toContainText(/\d/);

  const before = await cursorMs(page);
  await earlier.press("Enter");
  await expect(timeline.getByText("Nothing in this stretch of time")).toBeHidden();
  await expect(earlier).toHaveCount(0);
  await expect.poll(() => cursorMs(page)).toBeLessThan(before);
  await expect(timeline.getByRole("button", { name: "Whole range" }), "the zoom level is kept").toHaveAttribute("aria-pressed", "false");

  // The whole range always has something in it, so no buttons there.
  await timeline.getByRole("button", { name: "Whole range" }).press("Enter");
  await expect(timeline.getByText("Nothing in this stretch of time")).toHaveCount(0);
  expect(errors).toEqual([]);
});
