import { expect, test } from "@playwright/test";
import { login } from "./helpers";

const WORKSPACE = "/app/demo/local-demo";

test("admin creates, sees once and revokes an API token", async ({ page }) => {
  await login(page, "admin@controlhub.local", WORKSPACE);
  await page.getByRole("link", { name: "Tokens" }).click();
  await expect(page.getByRole("heading", { name: /Tokens/ })).toBeVisible();

  const name = `e2e-${Date.now()}`;
  await page.getByLabel("Name").fill(name);
  await page.getByRole("button", { name: "Create token" }).click();
  const secret = page.getByLabel("New token secret");
  await expect(secret).toHaveText(/^cha_/);
  await page.getByRole("button", { name: "Done" }).click();
  await expect(secret).toHaveCount(0);

  const row = page.getByRole("table", { name: "Tokens" }).getByRole("row").filter({ hasText: name });
  await expect(row).toContainText("active");
  page.once("dialog", (d) => void d.accept());
  await row.getByRole("button", { name: "Revoke" }).click();
  await expect(row).toContainText("revoked");
});

test("viewer has no token management", async ({ page }) => {
  await login(page, "viewer@controlhub.local", WORKSPACE);
  await expect(page.getByText("DEMO", { exact: true })).toBeVisible();
  await expect(page.getByRole("link", { name: "Tokens" })).toHaveCount(0);
  await page.goto(`${WORKSPACE}/tokens`);
  await expect(page.getByText("Token management requires the admin role.")).toBeVisible();
});
