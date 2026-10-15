import { expect, test, type Page } from "@playwright/test";
import { mapLayer, collectErrors, login } from "./helpers";

const WORKSPACE = "/app/demo/local-demo";

/** Opens a service in the right panel the way a user does without a pointer: from the table view. */
async function openService(page: Page, name: string) {
  await expect(page.getByText("Starting engine…")).toBeHidden();
  await (await mapLayer(page, "Table view")).check();
  await page.getByRole("button", { name: "Map layers" }).click();
  await page.getByRole("table").getByRole("button", { name, exact: true }).click();
  const inspector = page.getByRole("complementary", { name: "Inspector" });
  await expect(inspector.getByRole("heading", { name, exact: true })).toBeVisible();
  return inspector;
}

test("an admin restarts a catalog container from the System Map panel without a second approval", async ({ page }) => {
  test.setTimeout(150_000);
  const errors = collectErrors(page);
  await login(page, "admin@karbird.local", WORKSPACE);
  const inspector = await openService(page, "catalog");

  const control = inspector.getByRole("region", { name: "Control catalog" });
  await expect(control.getByRole("button", { name: "Scale replicas" })).toBeVisible();
  await control.getByRole("button", { name: "Restart container" }).click();
  const form = control.getByRole("form", { name: "Restart container" });
  await expect(form).toContainText("Verification:");
  await form.getByLabel("Target").selectOption("catalog-3");
  await form.getByLabel("Reason").fill(`e2e map restart ${Date.now()}`);
  await form.getByRole("button", { name: "Run" }).click();

  const recent = control.getByRole("list", { name: "Recent actions on catalog" });
  const item = recent.getByRole("listitem").filter({ hasText: "catalog-3" }).first();
  await expect(item).toBeVisible();
  await expect(item.getByRole("button", { name: /Approve/ })).toHaveCount(0);
  await expect(item).toContainText("succeeded", { timeout: 120_000 });
  expect(errors).toEqual([]);
});

test("an operator opens a container from its machine and can only request approval, then cancels", async ({ page }) => {
  test.setTimeout(90_000);
  const errors = collectErrors(page);
  await login(page, "operator@karbird.local", WORKSPACE);
  await expect(page.getByText("Starting engine…")).toBeHidden();
  await (await mapLayer(page, "Infrastructure")).check();
  await page.getByRole("button", { name: "Map layers" }).click();
  await page.getByRole("region", { name: "Machines" }).getByRole("button", { name: /^vps-worker/ }).click();

  const inspector = page.getByRole("complementary", { name: "Inspector" });
  await expect(inspector.getByRole("heading", { name: "vps-worker", exact: true })).toBeVisible();
  await inspector.getByRole("button", { name: "Open container catalog-2" }).click();
  await expect(inspector.getByRole("heading", { name: "catalog-2", exact: true })).toBeVisible();
  await expect(inspector.getByLabel("Container metrics")).toContainText("CPU of machine");

  const control = inspector.getByRole("region", { name: "Control catalog-2" });
  // A container offers only what is aimed at it: no scaling, no flags.
  await expect(control.getByRole("button", { name: "Scale replicas" })).toHaveCount(0);
  await control.getByRole("button", { name: "Restart container" }).click();
  const form = control.getByRole("form", { name: "Restart container" });
  await expect(form.getByLabel("Target")).toHaveCount(0);
  await expect(form).toContainText("catalog-2");
  await expect(form.getByRole("button", { name: "Run" })).toHaveCount(0);
  await form.getByLabel("Reason").fill(`e2e map request ${Date.now()}`);
  await form.getByRole("button", { name: "Request approval" }).click();

  const item = control.getByRole("list", { name: "Recent actions on catalog-2" }).getByRole("listitem").first();
  await expect(item).toContainText("proposed");
  await expect(item.getByRole("button", { name: /Approve/ })).toHaveCount(0);
  await item.getByRole("button", { name: "Cancel Restart container on catalog-2" }).click();
  await expect(item).toContainText("cancelled");

  // Back to the machine from the container card.
  await inspector.getByRole("button", { name: "vps-worker", exact: true }).click();
  await expect(inspector.getByRole("heading", { name: "vps-worker", exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});

test("viewers see the control section but cannot start anything; services without actions show none", async ({ page }) => {
  await login(page, "viewer@karbird.local", WORKSPACE);
  const inspector = await openService(page, "orders");
  const control = inspector.getByRole("region", { name: "Control orders" });
  await expect(control).toContainText("Your role in this project does not include running or proposing actions.");
  await expect(control.getByRole("button", { name: "Restart container" })).toHaveCount(0);

  await page.getByRole("table").getByRole("button", { name: "auth", exact: true }).click();
  await expect(inspector.getByRole("heading", { name: "auth", exact: true })).toBeVisible();
  await expect(inspector.getByRole("region", { name: /^Control/ })).toHaveCount(0);
});
