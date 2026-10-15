import { expect, test } from "./isolated-test";
import { login } from "./helpers";

const ISOLATED_PAGE = "/app/isolation/isolation-test/incidents";
const DEMO_PAGE = "/app/demo/local-demo/incidents";

test("project owner creates and pauses an incident rule from the queue", async ({ page }) => {
  const service = `incident-e2e-${Date.now()}`;
  await login(page, "isolation@karbird.local", ISOLATED_PAGE);
  await expect(page.getByRole("heading", { name: "Incident queue" })).toBeVisible();
  await page.getByRole("tab", { name: /rules/i }).click();
  const form = page.getByRole("form", { name: "Create incident rule" });
  await form.getByLabel("Service ID").fill(service);
  await form.getByLabel("Threshold (%)").fill("12");
  await form.getByLabel("Minimum requests per 5 min").fill("25");
  await form.getByRole("button", { name: "Create rule" }).click();

  const entry = page.getByRole("article").filter({ hasText: service });
  await expect(entry).toContainText("Error rate > 12%");
  await expect(entry).toContainText("Observation: unknown");
  await entry.getByRole("button", { name: "Pause" }).click();
  await expect(entry).toContainText("Paused");
});

test("viewer sees the incident queue without rule controls", async ({ page }) => {
  await login(page, "viewer@karbird.local", DEMO_PAGE);
  await page.getByRole("tab", { name: /rules/i }).click();
  await expect(page.getByRole("form", { name: "Create incident rule" })).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Incident queue" })).toBeVisible();
});
