import { expect, test } from "./isolated-test";
import { collectErrors } from "./helpers";

test.describe("landing page demo", () => {
  test("scrubbing the timeline back shows the system before the fault, and Live returns to now", async ({ page }) => {
    const errors = collectErrors(page);
    await page.goto("/");
    const demo = page.getByRole("group", { name: "Interactive demo" });
    await expect(demo).toBeVisible();
    await expect(demo.getByText("Demo · synthetic data")).toBeVisible();

    const map = demo.getByRole("img", { name: /Service map at/ });
    const live = demo.getByRole("button", { name: "Live" });
    await expect(live).toHaveAttribute("aria-pressed", "true");
    await expect(map).toHaveAttribute("aria-label", /Failing: orders/);

    const slider = demo.getByRole("slider", { name: "Replay time" });
    await slider.focus();
    await page.keyboard.press("Home");
    await expect(slider).toHaveAttribute("aria-valuetext", "−15:00");
    await expect(map).toHaveAttribute("aria-label", /All services healthy/);
    await expect(live).toHaveAttribute("aria-pressed", "false");

    await demo.getByRole("button", { name: /^orders, / }).click();
    await expect(demo.getByRole("button", { name: "orders, healthy" })).toHaveAttribute("aria-pressed", "true");

    await live.click();
    await expect(slider).toHaveAttribute("aria-valuetext", "now");
    await expect(demo.getByRole("button", { name: "orders, failing" })).toBeVisible();
    expect(errors).toEqual([]);
  });

  test("every service of the demo system is on the map, and selecting one shows its details", async ({ page }) => {
    await page.goto("/");
    const demo = page.getByRole("group", { name: "Interactive demo" });
    for (const name of ["gateway", "auth", "catalog", "orders", "inventory", "payments", "notifications"]) {
      await expect(demo.getByRole("button", { name: new RegExp(`^${name}, `) })).toBeVisible();
    }
    await expect(demo.getByRole("button", { name: /^External clients, / })).toBeVisible();
    await expect(demo.getByRole("button", { name: /^payment API, / })).toBeVisible();

    const payments = demo.getByRole("button", { name: /^payments, / });
    await payments.click();
    await expect(payments).toHaveAttribute("aria-pressed", "true");
    await expect(demo.getByRole("button", { name: /^orders, / })).toHaveAttribute("aria-pressed", "false");
    const service = demo.getByRole("definition").first();
    await expect(service).toHaveText("payments");
    await expect(demo.getByText("Demo · synthetic data")).toBeVisible();
  });

  test("the same moment always replays the same picture", async ({ page }) => {
    await page.goto("/");
    const demo = page.getByRole("group", { name: "Interactive demo" });
    const slider = demo.getByRole("slider", { name: "Replay time" });
    const map = demo.getByRole("img", { name: /Service map at/ });

    // The slider is server-rendered; wait until hydration attaches its change handler.
    await expect.poll(async () => {
      await slider.fill("569");
      await slider.fill("570");
      return map.getAttribute("aria-label");
    }).toMatch(/Degraded: orders/);
    const first = await demo.locator("svg circle").evaluateAll((cs) => cs.map((c) => `${c.getAttribute("cx")},${c.getAttribute("cy")}`).join(";"));

    await slider.fill("900");
    await expect(map).toHaveAttribute("aria-label", /Failing: orders/);
    await slider.fill("570");
    await expect(map).toHaveAttribute("aria-label", /Degraded: orders/);
    const second = await demo.locator("svg circle").evaluateAll((cs) => cs.map((c) => `${c.getAttribute("cx")},${c.getAttribute("cy")}`).join(";"));
    expect(second).toBe(first);
  });
});
