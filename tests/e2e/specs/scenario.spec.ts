import { expect, test } from "@playwright/test";
import { login } from "./helpers";

// Demo-only: uses the loopback fault-injection endpoint of the demo orders
// service (enabled by `just demo`). Always clears the fault afterwards.
const ORDERS = process.env.DEMO_ORDERS_URL ?? "http://127.0.0.1:7002";

async function setFault(latencyMs: number, errorRate: number) {
  const res = await fetch(`${ORDERS}/__demo/faults`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ latencyMs, errorRate }),
  });
  if (!res.ok) throw new Error(`fault injection unavailable (${res.status}); start the demo with just demo`);
}

test("injected errors in orders show up as a failing component and error traces", async ({ page }) => {
  test.setTimeout(120_000);
  await setFault(150, 0.6);
  try {
    await login(page, "admin@karbird.local", "/app/demo/local-demo");
    await page.getByRole("checkbox", { name: "Table view" }).check();
    const ordersRow = page.getByRole("row").filter({ has: page.getByRole("button", { name: "orders", exact: true }) });
    await expect(ordersRow).toContainText(/Degraded|Failing/, { timeout: 60_000 });

    await page.getByRole("checkbox", { name: "Table view" }).uncheck();
    await page.getByRole("checkbox", { name: "Only problems" }).check();
    await page.getByRole("checkbox", { name: "Only problems" }).uncheck();
    await page.getByPlaceholder("Find component…").fill("");

    const panel = page.getByRole("region", { name: "Signals" });
    await panel.getByRole("tab", { name: "Traces" }).click();
    await panel.getByLabel("Only errors").check();
    await expect(panel.getByRole("list", { name: "Traces" }).getByRole("button").first()).toBeVisible();
    await panel.getByRole("list", { name: "Traces" }).getByRole("button").first().click();
    await expect(panel.getByRole("list", { name: "Span waterfall" })).toBeVisible();
    await page.waitForTimeout(1500);
    await page.screenshot({ path: "../../test-results/e2e/fault-scenario.png" });
  } finally {
    await setFault(0, 0);
  }
});
