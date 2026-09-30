import { randomUUID } from "node:crypto";
import { expect, test, type Page } from "@playwright/test";
import { collectErrors, login, pageAlert, randomHex } from "./helpers";

const WORKSPACE = "/app/demo/local-demo";
const PASSWORD = `invite horse ${randomHex(4)}`;

async function openInviteLinks(page: Page) {
  await login(page, "admin@controlhub.local", WORKSPACE);
  await page.getByRole("link", { name: "Members & roles" }).click();
  await page.getByRole("tab", { name: "Invite links" }).click();
  await expect(page.getByRole("heading", { name: "Invite people with a link" })).toBeVisible();
}

async function createLink(page: Page, role: string, singleUse: boolean) {
  await page.getByLabel("Role for new members").selectOption({ label: role });
  if (singleUse) await page.getByLabel("Single use").check();
  await page.getByRole("button", { name: "Create invite link" }).click();
  const link = page.getByRole("status", { name: "New invite link" }).getByLabel("Invite link");
  await expect(link).toHaveText(/\/app\/invite#chv_[A-Za-z0-9_-]+$/);
  return (await link.textContent()) ?? "";
}

test("an admin shares an invite link; a newcomer creates an account from it and joins with the chosen role", async ({ page, browser }) => {
  const errors = collectErrors(page);
  await openInviteLinks(page);
  const link = await createLink(page, "Viewer", true);
  const secret = link.slice(link.indexOf("#") + 1);
  const active = page.getByRole("table", { name: "Active invite links" });
  await expect(active.getByRole("row").filter({ hasText: secret.slice(0, 10) })).toContainText("Viewer");

  const guest = await browser.newContext();
  const newcomer = await guest.newPage();
  const guestErrors = collectErrors(newcomer);
  const urls: string[] = [];
  newcomer.on("request", (r) => urls.push(r.url()));
  await newcomer.goto(link);
  await expect(newcomer.getByRole("heading", { level: 1, name: /^Join / })).toBeVisible();
  await expect(newcomer.getByText("invited you to join as Viewer")).toBeVisible();
  await expect(newcomer).toHaveURL(/\/app\/invite$/, { timeout: 5_000 });

  const email = `e2e-invitee-${randomUUID()}@example.test`;
  await newcomer.getByRole("link", { name: "Create an account" }).click();
  await expect(newcomer).toHaveURL(/\/app\/register\?next=%2Fapp%2Finvite$/);
  await newcomer.getByLabel("Email").fill(email);
  await newcomer.getByLabel("Password", { exact: true }).fill(PASSWORD);
  await newcomer.getByLabel("Repeat password").fill(PASSWORD);
  await newcomer.getByRole("button", { name: "Create account" }).click();

  // Back on the invite, now signed in: one click joins the project.
  await expect(newcomer).toHaveURL(/\/app\/invite$/);
  const join = newcomer.getByRole("button", { name: /^Join / });
  await expect(join).toBeVisible();
  await join.click();
  await expect(newcomer).toHaveURL(new RegExp(`${WORKSPACE}$`));
  await expect(newcomer.getByRole("link", { name: "Members & roles" })).toBeVisible();
  await expect(newcomer.getByRole("navigation", { name: "Workspace" }).getByRole("link", { name: "Tokens" })).toHaveCount(0);
  expect(urls.filter((u) => u.includes(secret)), "the secret never goes to a server in a URL").toEqual([]);

  // The link was single use: it is gone for the next person and from the admin's list.
  const late = await (await browser.newContext()).newPage();
  await late.goto(link);
  await expect(pageAlert(late)).toContainText("expired, was used up or was revoked");
  await page.reload();
  await page.getByRole("tab", { name: "Invite links" }).click();
  await expect(page.getByRole("table", { name: "Active invite links" })).not.toContainText(secret.slice(0, 10));

  await page.getByRole("tab", { name: /Members/ }).click();
  await expect(page.getByRole("table", { name: "Members" })).toContainText(email.split("@")[0]);
  expect(errors).toEqual([]);
  expect(guestErrors.filter((e) => !/401|Unauthorized/.test(e))).toEqual([]);
  await guest.close();
});

test("a revoked invite link stops working and an incomplete link explains itself", async ({ page, browser }) => {
  await openInviteLinks(page);
  const link = await createLink(page, "Viewer", false);
  const prefix = link.slice(link.indexOf("#") + 1).slice(0, 10);
  const row = page.getByRole("table", { name: "Active invite links" }).getByRole("row").filter({ hasText: prefix });
  page.once("dialog", (d) => void d.accept());
  await row.getByRole("button", { name: /^Revoke invite / }).click();
  await expect(page.getByRole("table", { name: "Active invite links" })).not.toContainText(prefix);

  const other = await (await browser.newContext()).newPage();
  await other.goto(link);
  await expect(pageAlert(other)).toContainText("expired, was used up or was revoked");
  const fresh = await (await browser.newContext()).newPage();
  await fresh.goto("/app/invite");
  await expect(pageAlert(fresh)).toHaveText("This invite link is incomplete. Open the full link you were sent.");
});
