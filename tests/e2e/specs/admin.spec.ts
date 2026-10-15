import { expect, test, type Page } from "./isolated-test";
import { collectErrors, login, randomHex } from "./helpers";

const ADMIN = "/app/admin";

// The tests grant, disable and enable sysmember@; run them one after another.
test.describe.configure({ mode: "serial" });

/** Answers the next window.prompt / window.confirm. */
function answerNextDialog(page: Page, text?: string) {
  page.once("dialog", (d) => void d.accept(text));
}

function projectRow(page: Page, name: string) {
  return page.getByRole("table", { name: "All projects" }).getByRole("row").filter({ hasText: name });
}

test("a Super Admin creates, renames, extends and deletes a project from the admin panel", async ({ page }) => {
  const errors = collectErrors(page);
  const name = `[test] E2E ${randomHex(3)}`;
  const renamed = `${name} renamed`;
  await login(page, "superadmin@karbird.local", "/app");
  await page.getByRole("link", { name: "karbird admin" }).click();
  await expect(page).toHaveURL(/\/app\/admin$/);
  await expect(page.getByRole("heading", { name: "karbird administration" })).toBeVisible();
  await expect(page.getByText("Super Admin", { exact: true })).toBeVisible();
  await expect(page.getByRole("definition").first()).toHaveText(/\d+/);

  await page.getByRole("tab", { name: "Projects" }).click();
  const demoRow = page.getByRole("table", { name: "All projects" }).getByRole("row").filter({ has: page.getByText(/^demo · created/) });
  await expect(demoRow.getByRole("button", { name: /^Delete / })).toBeDisabled();
  await expect(demoRow.getByRole("button", { name: /^Delete / })).toHaveAttribute("title", "The demo project cannot be deleted.");
  const form = page.getByRole("form", { name: "New project" });
  await form.getByLabel("Project name").fill(name);
  await form.getByLabel("Owner email (optional)").fill("sysmember@karbird.local");
  await form.getByRole("button", { name: "Create project" }).click();
  const row = projectRow(page, name);
  await expect(row).toContainText("sysmember@karbird.local");
  await expect(row).toContainText("production");

  answerNextDialog(page, renamed);
  await row.getByRole("button", { name: `Rename ${name}` }).click();
  const renamedRow = projectRow(page, renamed);
  await expect(renamedRow).toBeVisible();

  answerNextDialog(page, "staging");
  await renamedRow.getByRole("button", { name: `Add environment to ${renamed}` }).click();
  await expect(renamedRow).toContainText("staging");

  await renamedRow.getByRole("button", { name: "Members" }).click();
  const members = page.getByRole("table", { name: `Members of ${renamed}` });
  await expect(members).toContainText("sysmember@karbird.local");
  await expect(members.getByLabel("Project role for System member")).toHaveValue("owner");

  // A wrong confirmation keeps the project; the exact name deletes it.
  answerNextDialog(page, "nope");
  await renamedRow.getByRole("button", { name: `Delete ${renamed}` }).click();
  await expect(renamedRow).toBeVisible();
  answerNextDialog(page, renamed);
  await renamedRow.getByRole("button", { name: `Delete ${renamed}` }).click();
  await expect(projectRow(page, renamed)).toHaveCount(0);

  await page.getByRole("tab", { name: "Audit log" }).click();
  const log = page.getByRole("table", { name: "Audit log" });
  await expect(log.getByRole("row").filter({ hasText: "Deleted project" }).first()).toContainText("superadmin@karbird.local");
  expect(errors).toEqual([]);
});

test("granting administrator access shows the panel to that account at once; removing it hides it", async ({ page, browser }) => {
  await login(page, "superadmin@karbird.local", ADMIN);
  await page.getByRole("tab", { name: "Administrators" }).click();
  const form = page.getByRole("form", { name: "Add administrator" });
  await form.getByLabel("Account email").fill("sysmember@karbird.local");
  await form.getByLabel("Administrator role").selectOption({ label: "Auditor" });
  await form.getByRole("button", { name: "Add administrator" }).click();
  const admins = page.getByRole("table", { name: "Administrators" });
  await expect(admins.getByLabel("Administrator role for System member")).toBeVisible();
  await expect(admins.getByLabel("Administrator role for Test super admin")).toBeDisabled();

  const ctx = await browser.newContext();
  const member = await ctx.newPage();
  await login(member, "sysmember@karbird.local", "/app");
  await member.getByRole("link", { name: "karbird admin" }).click();
  await expect(member.getByRole("heading", { name: "karbird administration" })).toBeVisible();
  await expect(member.getByRole("tab", { name: "Audit log" })).toBeVisible();
  await member.getByRole("tab", { name: "Projects" }).click();
  await expect(member.getByRole("table", { name: "All projects" })).toBeVisible();
  await expect(member.getByRole("form", { name: "New project" })).toHaveCount(0);

  answerNextDialog(page);
  await admins.getByRole("button", { name: "Remove administrator System member" }).click();
  await expect(admins.getByLabel("Administrator role for System member")).toHaveCount(0);
  await member.reload();
  await expect(member.getByText("This page does not exist or you do not have access to it.")).toBeVisible();
  await ctx.close();
});

test("an Auditor sees every section but no controls that change anything", async ({ page }) => {
  await login(page, "auditor@karbird.local", ADMIN);
  const tabs = page.getByRole("tablist", { name: "Administration" }).getByRole("tab");
  await expect(tabs).toHaveText(["Overview", "Projects", "Accounts", "Administrators", "Roles", "Audit log"]);
  await page.getByRole("tab", { name: "Projects" }).click();
  await expect(page.getByRole("table", { name: "All projects" })).toBeVisible();
  await expect(page.getByRole("button", { name: /^Delete / })).toHaveCount(0);
  await page.getByRole("tab", { name: "Accounts" }).click();
  await expect(page.getByRole("table", { name: "Accounts" })).toContainText("sysmember@karbird.local");
  await expect(page.getByRole("button", { name: /^Disable / })).toHaveCount(0);
  await page.getByRole("tab", { name: "Roles" }).click();
  await expect(page.getByLabel("View the audit log of every project for Auditor")).toBeDisabled();
  await expect(page.getByRole("form", { name: "New administrator role" })).toHaveCount(0);
});

test("accounts without administrator access see neither the link nor the panel", async ({ page }) => {
  await login(page, "viewer@karbird.local", "/app/account");
  await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
  await expect(page.getByRole("link", { name: "karbird admin" })).toHaveCount(0);
  await page.goto(ADMIN);
  await expect(page.getByText("This page does not exist or you do not have access to it.")).toBeVisible();
  await expect(page.getByRole("tablist", { name: "Administration" })).toHaveCount(0);
});

test("disabling an account signs it out; enabling lets it back in", async ({ page, browser }) => {
  const ctx = await browser.newContext();
  const member = await ctx.newPage();
  await login(member, "sysmember@karbird.local", "/app/account");

  await login(page, "superadmin@karbird.local", ADMIN);
  await page.getByRole("tab", { name: "Accounts" }).click();
  await page.getByLabel("Find account").fill("sysmember");
  const row = page.getByRole("table", { name: "Accounts" }).getByRole("row").filter({ hasText: "sysmember@karbird.local" });
  await expect(row.getByRole("button", { name: "Disable System member" })).toBeVisible();

  answerNextDialog(page);
  await row.getByRole("button", { name: "Disable System member" }).click();
  await expect(row.getByText("Disabled", { exact: true })).toBeVisible();
  await member.reload();
  await expect(member).toHaveURL(/\/app\/login/);

  await row.getByRole("button", { name: "Enable System member" }).click();
  await expect(row.getByText("Active", { exact: true })).toBeVisible();
  await login(member, "sysmember@karbird.local", "/app/account");
  await expect(member).toHaveURL(/\/app\/account/);
  await ctx.close();
});
