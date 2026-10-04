import { expect, test, type Page } from "@playwright/test";
import { collectErrors, ingestSpans, login, randomHex } from "./helpers";

const WORKSPACE = "/app/isolation/isolation-test";
const MS = 1_000_000n;

/** Cursor time of the timeline ruler in ms since midnight, from its aria-valuetext. */
async function cursorMs(page: Page): Promise<number> {
  const text = (await page.getByLabel("Timeline").getByRole("slider", { name: "Selected time" }).getAttribute("aria-valuetext")) ?? "";
  const m = /(\d+):(\d+):(\d+)/.exec(text);
  return m ? ((Number(m[1]) * 60 + Number(m[2])) * 60 + Number(m[3])) * 1000 : Number.NaN;
}

/** Pauses, puts the cursor about ten minutes into the loaded quarter hour and zooms in to well under a second. */
async function zoomIntoQuietStretch(page: Page) {
  const timeline = page.getByLabel("Timeline");
  await expect(page.getByText("Starting engine…")).toBeHidden();
  await timeline.getByRole("button", { name: "Pause live" }).click();
  const ruler = timeline.getByRole("slider", { name: "Selected time" });
  await ruler.focus();
  await page.keyboard.press("Home");
  for (let i = 0; i < 6; i++) await page.keyboard.press("Shift+ArrowRight");
  const zoomIn = timeline.getByRole("button", { name: "Zoom in time" });
  for (let i = 0; i < 12; i++) await zoomIn.click();
  await expect(timeline.getByRole("button", { name: "Whole range" })).toHaveAttribute("aria-pressed", "false");
}

test("a zoomed-in stretch with nothing in it offers the nearest item on each side", async ({ page }) => {
  const errors = collectErrors(page);
  const service = `quiet-${randomHex(4)}`;
  const now = BigInt(Date.now()) * MS;
  const minute = 60_000n * MS;
  const span = (at: bigint) => ({ traceId: randomHex(16), spanId: randomHex(8), name: "GET /quiet", kind: 2, startNs: at, endNs: at + 30n * MS });
  // One request before and one after the stretch the test zooms into (about 5 minutes ago).
  await ingestSpans(service, [span(now - 9n * minute), span(now - 2n * minute)]);

  await login(page, "isolation@karbird.local", WORKSPACE);
  const timeline = page.getByLabel("Timeline");
  const earlier = timeline.getByRole("button", { name: /^Go to the nearest earlier item/ });
  const later = timeline.getByRole("button", { name: /^Go to the nearest later item/ });

  // Telemetry reaches the workspace through the pipeline; reload until both spans are loaded.
  await expect
    .poll(
      async () => {
        await zoomIntoQuietStretch(page);
        const shown = await later.waitFor({ timeout: 3_000 }).then(
          () => true,
          () => false,
        );
        if (shown && (await earlier.isVisible())) return true;
        await page.reload();
        return false;
      },
      { timeout: 60_000, intervals: [2_000] },
    )
    .toBe(true);

  await expect(timeline.getByText("Nothing in this stretch of time")).toBeVisible();
  await expect(earlier).toContainText(/\d/);
  await expect(later).toContainText(/\d/);

  const before = await cursorMs(page);
  await earlier.click();
  await expect(timeline.getByText("Nothing in this stretch of time")).toBeHidden();
  await expect(earlier).toHaveCount(0);
  await expect.poll(() => cursorMs(page)).toBeLessThan(before);
  await expect(timeline.getByRole("button", { name: "Whole range" }), "the zoom level is kept").toHaveAttribute("aria-pressed", "false");

  // The whole range always has something in it, so no buttons there.
  await timeline.getByRole("button", { name: "Whole range" }).click();
  await expect(timeline.getByText("Nothing in this stretch of time")).toHaveCount(0);
  expect(errors).toEqual([]);
});
