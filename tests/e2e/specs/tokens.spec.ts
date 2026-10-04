import { expect, test } from "@playwright/test";
import { login } from "./helpers";

const WORKSPACE = "/app/demo/local-demo";

test("admin creates, sees once and revokes an API token", async ({ page }) => {
  await login(page, "admin@karbird.local", WORKSPACE);
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

test("admin creates a non-expiring ingestion token, rotates it with a grace period and cleans up", async ({ page }) => {
  await login(page, "admin@karbird.local", `${WORKSPACE}/tokens`);
  await expect(page.getByRole("heading", { level: 1, name: "Tokens" })).toBeVisible();
  const name = `e2e-ingest-${Date.now()}`;
  const form = page.locator("form");
  await form.getByLabel("Kind").selectOption("ingest");
  await expect(form.getByLabel(/Expires in days/)).toHaveValue("");
  await expect(form.getByText("(empty = never)")).toBeVisible();
  await form.getByLabel("Name").fill(name);
  await form.getByRole("button", { name: "Create token" }).click();

  const notice = page.getByRole("status").filter({ hasText: "Copy it now" });
  await expect(notice).toContainText(`New ingestion token “${name}”`);
  const secret = page.getByLabel("New token secret");
  await expect(secret).toHaveText(/^chi_[A-Za-z0-9_-]{20,}$/);
  const first = (await secret.textContent()) ?? "";
  await page.getByRole("button", { name: "Done" }).click();

  const rows = page.getByRole("table", { name: "Tokens" }).getByRole("row").filter({ hasText: name });
  await expect(rows).toHaveCount(1);
  await expect(rows).toContainText("Ingestion");
  await expect(rows).toContainText("never");
  await expect(rows).toContainText(first.slice(0, 8));
  await expect(page.getByRole("table", { name: "Tokens" })).not.toContainText(first);

  page.once("dialog", (d) => {
    expect(d.message()).toContain("keeps working for 1 hour");
    void d.accept();
  });
  await rows.getByRole("button", { name: "Rotate" }).click();
  await expect(secret).toHaveText(/^chi_/);
  expect(await secret.textContent()).not.toBe(first);
  await page.getByRole("button", { name: "Done" }).click();
  // The old token stays active for the grace period next to its replacement.
  await expect(rows).toHaveCount(2);
  await expect(rows.filter({ hasText: "active" })).toHaveCount(2);

  for (let i = 0; i < 2; i++) {
    page.once("dialog", (d) => void d.accept());
    await rows.filter({ hasText: "active" }).first().getByRole("button", { name: "Revoke" }).click();
    await expect(rows.filter({ hasText: "revoked" })).toHaveCount(i + 1);
  }
  await expect(rows.getByRole("button", { name: "Revoke" })).toHaveCount(0);
});

test("dismissing the confirmation does not revoke; server-side validation errors are shown", async ({ page }) => {
  await login(page, "admin@karbird.local", `${WORKSPACE}/tokens`);
  const name = `e2e-keep-${Date.now()}`;
  await page.getByLabel("Name").fill(name);
  await page.getByRole("button", { name: "Create token" }).click();
  await page.getByRole("button", { name: "Done" }).click();
  const row = page.getByRole("table", { name: "Tokens" }).getByRole("row").filter({ hasText: name });
  page.once("dialog", (d) => void d.dismiss());
  await row.getByRole("button", { name: "Revoke" }).click();
  await expect(row).toContainText("active");

  // Only spaces pass the browser pattern but not the server's name rule.
  await page.getByLabel("Name").fill("   ");
  await page.getByRole("button", { name: "Create token" }).click();
  await expect(page.locator("main").getByRole("alert")).toContainText(/name/i);

  page.once("dialog", (d) => void d.accept());
  await row.getByRole("button", { name: "Revoke" }).click();
  await expect(row).toContainText("revoked");
});

test("viewer has no token management", async ({ page }) => {
  await login(page, "viewer@karbird.local", WORKSPACE);
  await expect(page.getByText("DEMO", { exact: true })).toBeVisible();
  await expect(page.getByRole("link", { name: "Tokens" })).toHaveCount(0);
  await page.goto(`${WORKSPACE}/tokens`);
  await expect(page.getByText("Your role in this project does not include access to tokens.")).toBeVisible();
});

test("operator sees tokens read-only", async ({ page }) => {
  await login(page, "operator@karbird.local", WORKSPACE);
  await page.getByRole("link", { name: "Tokens" }).click();
  await expect(page.getByRole("heading", { name: "Tokens" })).toBeVisible();
  await expect(page.getByRole("table", { name: "Tokens" })).toBeVisible();
  await expect(page.getByRole("note")).toContainText("Create, rotate and revoke tokens");
  await expect(page.getByRole("button", { name: "Create token" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Revoke" })).toHaveCount(0);
});
