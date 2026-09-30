import { expect, test } from "@playwright/test";
import { collectErrors, ownerAccount } from "./helpers";

// The developer's own account from .controlhub/owner.env, signing in on the page they use.
const owner = ownerAccount();

test("the owner signs in on /app/login and lands in the demo workspace", async ({ page }) => {
  test.skip(!owner, "no .controlhub/owner.env on this machine (bootstrap requires it)");
  if (!owner) return;
  const errors = collectErrors(page);
  await page.goto("/app/login");

  // A wrong password first: a clear message and no way in. The success below resets the counter.
  await page.getByLabel("Email").fill(owner.email);
  await page.getByLabel("Password").fill(`${owner.password}-wrong`);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("alert").filter({ hasText: "Invalid email or password." })).toBeVisible();
  await expect(page).toHaveURL(/\/app\/login/);

  await page.getByLabel("Password").fill(owner.password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page).toHaveURL(/\/app\/demo\/local-demo/);
  await expect(page.getByText("Demo system")).toBeVisible();
  await expect(page.getByText("Starting engine…")).toBeHidden();

  // The session survives a reload.
  await page.reload();
  await expect(page).toHaveURL(/\/app\/demo\/local-demo/);
  expect(errors.filter((e) => !e.includes("401"))).toEqual([]);
});

test("the login page does not point to generated demo accounts", async ({ page }) => {
  await page.goto("/app/login");
  await expect(page.getByRole("heading", { name: "Sign in to ControlHub" })).toBeVisible();
  await expect(page.getByText("local-credentials.json")).toHaveCount(0);
  await expect(page.getByText(".controlhub/owner.env")).toBeVisible();
});
