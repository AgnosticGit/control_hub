import { randomUUID } from "node:crypto";
import { expect, test, type Browser, type Page } from "@playwright/test";
import { collectErrors, pageAlert, randomHex } from "./helpers";

const PASSWORD = `account horse ${randomHex(4)}`;

async function register(page: Page) {
  const email = `e2e-account-${randomUUID()}@example.test`;
  await page.goto("/app/register");
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await page.getByLabel("Repeat password").fill(PASSWORD);
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page).toHaveURL(/\/app$/);
  await page.getByLabel("Organization name").fill(`E2E org ${randomHex(3)}`);
  await page.getByLabel("First project").fill(`E2E project ${randomHex(3)}`);
  await page.getByRole("button", { name: "Create workspace" }).click();
  await expect(page).toHaveURL(/\/app\/onboarding\?project=p-[0-9a-f]{32}$/);
  return email;
}

async function signIn(browser: Browser, email: string, password: string) {
  const page = await (await browser.newContext()).newPage();
  await page.goto("/app/login");
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in" }).click();
  return page;
}

test("the account home shows the user and their projects; the profile name can be changed", async ({ page }) => {
  const errors = collectErrors(page);
  const email = await register(page);
  await page.goto("/app/account");
  await expect(page.getByRole("heading", { level: 1, name: "Your account" })).toBeVisible();
  await expect(page.getByText(`Welcome, ${email.split("@")[0]}`)).toBeVisible();
  const projects = page.getByRole("region", { name: "Your projects" });
  await expect(projects.getByRole("link", { name: /^Open / })).toHaveCount(1);

  // The setup card offers onboarding until it is dismissed.
  const setup = page.getByRole("region", { name: "Setup" });
  await expect(setup.getByRole("link", { name: "Continue setup" })).toHaveAttribute("href", "/app/onboarding");
  await setup.getByRole("button", { name: "Dismiss" }).click();
  await expect(setup).toHaveCount(0);

  const sections = page.getByRole("navigation", { name: "Account sections" });
  await sections.getByRole("link", { name: "Personal info" }).click();
  await expect(page.getByLabel("Email")).toHaveValue(email);
  await expect(page.getByLabel("Email")).not.toBeEditable();
  const name = `Ada ${randomHex(2)}`;
  await page.getByLabel("Display name").fill(name);
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByRole("status")).toHaveText("Saved.");
  await sections.getByRole("link", { name: "Home" }).click();
  await expect(page.getByText(`Welcome, ${name}`)).toBeVisible();
  await page.reload();
  await expect(page.getByText(`Welcome, ${name}`)).toBeVisible();
  await expect(page.getByRole("region", { name: "Setup" })).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("account selects share the same styled control and profile fields have even spacing", async ({ page }) => {
  await register(page);
  for (const route of ["/app/account/projects", "/app/account/people"]) {
    await page.goto(route);
    const select = page.getByRole("combobox").first();
    await expect(select).toBeVisible();
    const style = await select.evaluate((element) => {
      const computed = getComputedStyle(element);
      return { height: computed.height, radius: computed.borderRadius, paddingRight: computed.paddingRight };
    });
    expect(style).toEqual({ height: "36px", radius: "8px", paddingRight: "36px" });
  }
  await page.goto("/app/account/profile");
  for (const name of ["Display name", "Email"]) {
    const input = page.getByLabel(name);
    const spacing = await input.evaluate((element) => getComputedStyle(element.parentElement ?? element).rowGap);
    expect(spacing).toBe("6px");
  }
});

test("security: other sessions can be signed out and the password changed", async ({ page, browser }) => {
  const email = await register(page);
  const other = await signIn(browser, email, PASSWORD);
  await expect(other).not.toHaveURL(/\/app\/login/);

  await page.goto("/app/account/security");
  const sessions = page.getByRole("table", { name: "Active sessions" });
  await expect(sessions.getByRole("row")).toHaveCount(3); // header + 2 sessions
  await expect(sessions.getByText("This browser")).toHaveCount(1);
  await page.getByRole("button", { name: "Sign out all other sessions" }).click();
  await expect(sessions.getByRole("row")).toHaveCount(2);
  await other.reload();
  await expect(other).toHaveURL(/\/app\/login/);

  await page.getByLabel("Current password").fill("not my password");
  await page.getByLabel("New password", { exact: true }).fill(`${PASSWORD} v2`);
  await page.getByLabel("Repeat new password").fill(`${PASSWORD} v2`);
  await page.getByRole("button", { name: "Change password" }).click();
  await expect(pageAlert(page)).toHaveText("Current password is wrong.");

  await page.getByLabel("Current password").fill(PASSWORD);
  await page.getByLabel("New password", { exact: true }).fill(`${PASSWORD} v2`);
  await page.getByLabel("Repeat new password").fill(`${PASSWORD} v3`);
  await page.getByRole("button", { name: "Change password" }).click();
  await expect(pageAlert(page)).toHaveText("New passwords do not match.");

  await page.getByLabel("Repeat new password").fill(`${PASSWORD} v2`);
  await page.getByRole("button", { name: "Change password" }).click();
  await expect(page.getByRole("status")).toHaveText("Password changed. Your other sessions were signed out.");
  const again = await signIn(browser, email, `${PASSWORD} v2`);
  await expect(again).not.toHaveURL(/\/app\/login/);
});

test("projects: create one, rename it, and the last owner cannot leave", async ({ page }) => {
  await register(page);
  await page.goto("/app/account/projects");
  const name = `Side project ${randomHex(3)}`;
  const form = page.getByRole("form", { name: "New project" });
  await form.getByLabel("Organization").selectOption({ index: 1 });
  await form.getByLabel("Project name").fill(name);
  await form.getByRole("button", { name: "Create project" }).click();
  await expect(page).toHaveURL(/\/app\/p-[0-9a-f]{32}\/e-[0-9a-f]{32}\/integration$/);
  await expect(page.getByRole("complementary", { name: "Sidebar" }).getByText(name)).toBeVisible();

  await page.goto("/app/account/projects");
  const table = page.getByRole("table", { name: "Your projects" });
  await expect(table.getByRole("row")).toHaveCount(3);
  const renamed = `${name} v2`;
  page.once("dialog", (d) => void d.accept(renamed));
  await table.getByRole("button", { name: `Rename ${name}` }).click();
  await expect(table.getByRole("button", { name: `Rename ${renamed}` })).toBeVisible();

  page.once("dialog", (d) => void d.accept());
  await table.getByRole("button", { name: `Leave ${renamed}` }).click();
  await expect(pageAlert(page)).toContainText("last owner");
  await expect(table.getByRole("row")).toHaveCount(3);
});
