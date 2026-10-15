import { randomUUID } from "node:crypto";
import { expect, test, type Page } from "@playwright/test";
import { collectErrors, randomHex } from "./helpers";

const PASSWORD = `onboard horse ${randomHex(4)}`;

async function register(page: Page) {
  await page.goto("/app/register");
  await page.getByLabel("Email").fill(`e2e-onboard-${randomUUID()}@example.test`);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByLabel("Repeat password").fill(PASSWORD);
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page).toHaveURL(/\/app$/);
  await page.getByLabel("Organization name").fill(`E2E onboarding ${randomHex(3)}`);
  await page.getByLabel("First project").fill(`Onboarding project ${randomHex(3)}`);
  await page.getByRole("button", { name: "Create workspace" }).click();
  await expect(page).toHaveURL(/\/app\/onboarding\?project=p-[0-9a-f]{32}$/);
  await expect(page.getByRole("heading", { level: 1, name: "Connect your system" })).toBeVisible();
}

test("every onboarding step can be skipped, and the invite step creates a working link", async ({ page }) => {
  const errors = collectErrors(page);
  await register(page);
  await page.getByRole("button", { name: "Skip this step" }).click();
  await expect(page).toHaveURL(/\/app\/onboarding\?step=invite&project=p-[0-9a-f]{32}$/);
  await expect(page.getByRole("heading", { level: 1, name: "Invite your team" })).toBeVisible();

  await page.getByRole("button", { name: "Create invite link" }).click();
  await expect(page.getByRole("status", { name: "New invite link" }).getByLabel("Invite link")).toHaveText(/\/app\/invite#chv_/);
  await page.getByRole("button", { name: "Continue" }).click();

  await expect(page.getByRole("heading", { level: 1, name: "You're all set" })).toBeVisible();
  await expect(page.getByText("No telemetry yet.", { exact: false })).toBeVisible();
  await page.getByRole("button", { name: "Go to your account" }).click();
  await expect(page).toHaveURL(/\/app\/account$/);
  await expect(page.getByRole("heading", { level: 1, name: "Your account" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Setup" })).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("leaving onboarding never blocks the app; Finish setup brings it back until it is skipped", async ({ page }) => {
  await register(page);
  // Leaving setup opens the environment picker; it never blocks the workspace.
  await page.getByRole("link", { name: "karbird" }).click();
  await expect(page).toHaveURL(/\/app$/);
  await page.getByRole("link", { name: /Development/ }).click();
  await expect(page).toHaveURL(/\/app\/p-[0-9a-f]{32}\/e-[0-9a-f]{32}$/);
  const finish = page.getByRole("link", { name: "Finish setup" });
  await expect(finish).toBeVisible();

  await finish.click();
  await expect(page).toHaveURL(/\/app\/onboarding\?project=p-[0-9a-f]{32}$/);
  await expect(page.getByRole("heading", { level: 1, name: "Connect your system" })).toBeVisible();
  await page.getByRole("button", { name: "Skip setup" }).click();
  await expect(page).toHaveURL(/\/app\/p-[0-9a-f]{32}\/e-[0-9a-f]{32}$/);
  await expect(page.getByRole("link", { name: "Finish setup" })).toHaveCount(0);
  await page.reload();
  await expect(page.getByRole("link", { name: "Your account" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Finish setup" })).toHaveCount(0);
});
