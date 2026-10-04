import { expect, test, type Page } from "@playwright/test";
import { collectErrors, login, randomHex } from "./helpers";

const WORKSPACE = "/app/demo/local-demo";
const SETTINGS = `${WORKSPACE}/settings`;

// Both tests change member-a's role; run them one after another.
test.describe.configure({ mode: "serial" });

/** The UI shows a change at once; wait until the server has stored it before another user looks. */
async function saved(page: Page, change: () => Promise<unknown>) {
  const stored = page.waitForResponse((r) => r.request().method() === "PATCH" && /\/api\/v1\/projects\/demo\/(members|roles)\//.test(r.url()));
  await change();
  expect((await stored).status()).toBe(200);
}

async function selectedRole(page: Page, member: string) {
  return page.getByLabel(`Role for ${member}`).evaluate((s: HTMLSelectElement) => s.selectedOptions[0]?.textContent ?? "");
}

test("admin creates a role with checkboxes, assigns it, and the member sees exactly what it allows", async ({ page, browser }) => {
  const errors = collectErrors(page);
  const role = `[test] Manager ${randomHex(3)}`;
  await login(page, "admin@karbird.local", WORKSPACE);
  await page.getByRole("link", { name: "Members & roles" }).click();
  await expect(page.getByRole("heading", { name: "Members & roles" })).toBeVisible();

  // Roles tab: create the template and tick its permissions in the matrix.
  await page.getByRole("tab", { name: "Roles" }).click();
  await page.getByLabel("Role name").fill(role);
  await page.getByRole("button", { name: "Create role" }).click();
  const matrix = page.getByRole("table", { name: "Roles" });
  await expect(matrix.getByRole("columnheader", { name: new RegExp(role.replace(/[[\]]/g, "\\$&")) })).toBeVisible();
  const telemetry = page.getByLabel(`View map, timeline, traces and logs for ${role}`);
  await saved(page, () => telemetry.check());
  await expect(telemetry).toBeChecked();
  const tokens = page.getByLabel(`View tokens for ${role}`);
  await saved(page, () => tokens.check());
  await expect(tokens).toBeChecked();
  await expect(page.getByLabel("View tokens for Admin")).toBeDisabled();

  // Members tab: give Member A the new role.
  await page.getByRole("tab", { name: /Members/ }).click();
  const members = page.getByRole("table", { name: "Members" });
  await expect(members.getByRole("row").filter({ hasText: "(you)" })).toBeVisible();
  await expect(page.getByLabel("Role for Test admin")).toBeDisabled();
  await saved(page, () => page.getByLabel("Role for Member A").selectOption({ label: role }));
  await expect.poll(() => selectedRole(page, "Member A")).toBe(role);

  // Member A sees Tokens (read-only) but not Actions or Members & roles.
  const memberContext = await browser.newContext();
  const member = await memberContext.newPage();
  await login(member, "member-a@karbird.local", WORKSPACE);
  const nav = member.getByRole("navigation", { name: "Workspace" });
  await expect(nav.getByRole("link", { name: "Tokens" })).toBeVisible();
  await expect(nav.getByRole("link", { name: "Actions" })).toHaveCount(0);
  await expect(nav.getByRole("link", { name: "Members & roles" })).toHaveCount(0);
  await nav.getByRole("link", { name: "Tokens" }).click();
  await expect(member.getByRole("table", { name: "Tokens" })).toBeVisible();
  await expect(member.getByRole("button", { name: "Create token" })).toHaveCount(0);

  // Fine-tuning one box turns the role into an individual set based on it.
  const memberRow = members.getByRole("row").filter({ hasText: "member-a@karbird.local" });
  await memberRow.getByRole("button", { name: "Permissions" }).click();
  await saved(page, () => page.getByRole("group", { name: "Permissions of Member A" }).getByLabel("View members").check());
  await expect.poll(() => selectedRole(page, "Member A")).toBe(`Individual, based on ${role}`);

  await member.reload();
  await expect(nav.getByRole("link", { name: "Members & roles" })).toBeVisible();
  await nav.getByRole("link", { name: "Members & roles" }).click();
  await expect(member.getByLabel("Role for Member B")).toBeDisabled();
  await expect(member.getByRole("button", { name: "Remove Member B" })).toBeDisabled();
  await memberContext.close();

  // Clean up through the UI: back to Viewer, then delete the role.
  await saved(page, () => page.getByLabel("Role for Member A").selectOption({ label: "Viewer" }));
  await expect.poll(() => selectedRole(page, "Member A")).toBe("Viewer");
  await page.getByRole("tab", { name: "Roles" }).click();
  page.once("dialog", (d) => void d.accept());
  await page.getByRole("button", { name: `Delete ${role}` }).click();
  await expect(page.getByRole("button", { name: `Delete ${role}` })).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("a viewer sees members and roles read-only", async ({ page }) => {
  await login(page, "member-b@karbird.local", SETTINGS);
  await expect(page.getByRole("heading", { name: "Members & roles" })).toBeVisible();
  await expect(page.getByLabel("Role for Member A")).toBeDisabled();
  await expect(page.getByRole("button", { name: "Remove Member A" })).toBeDisabled();
  await page.getByRole("tab", { name: "Roles" }).click();
  await expect(page.getByRole("table", { name: "Roles" })).toBeVisible();
  await expect(page.getByLabel("Role name")).toHaveCount(0);
  await expect(page.getByLabel("View tokens for Operator")).toBeDisabled();
});

test("another tenant's members page looks absent", async ({ page }) => {
  await login(page, "viewer@karbird.local", WORKSPACE);
  await expect(page.getByRole("navigation", { name: "Workspace" }).getByRole("link", { name: "Members & roles" })).toBeVisible();
  await page.goto("/app/isolation/isolation-test/settings");
  await expect(page.getByText("This environment does not exist or you do not have access to it.")).toBeVisible();
});
