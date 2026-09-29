import { expect, test } from "@playwright/test";
import { login } from "./helpers";

const WORKSPACE = "/app/demo/local-demo";

test("operator proposes, a different admin approves, the executor runs and verifies", async ({ browser }) => {
  test.setTimeout(150_000);
  const reason = `e2e ${Date.now()}`;

  const opCtx = await browser.newContext();
  const op = await opCtx.newPage();
  await login(op, "operator@controlhub.local", WORKSPACE);
  await op.getByRole("link", { name: "Actions" }).click();
  await expect(op.getByRole("heading", { name: /Actions/ })).toBeVisible();
  const form = op.getByRole("form", { name: "Propose action" });
  await form.getByLabel("Action").selectOption("demo.faults.clear");
  await form.getByLabel("Target").selectOption("gateway");
  await form.getByLabel(/Reason/).fill(reason);
  await form.getByRole("button", { name: "Propose plan" }).click();

  const detail = op.getByRole("region", { name: "Plan detail" });
  await expect(detail.getByRole("heading", { level: 2 })).toContainText("proposed");
  await expect(detail).toContainText("✓ demo environment");
  const opRow = op.getByRole("table", { name: "Action plans" }).getByRole("row").filter({ hasText: "operator@controlhub.local" }).first();
  await expect(opRow.getByRole("button", { name: "Approve" })).toHaveCount(0);

  const adminCtx = await browser.newContext();
  const admin = await adminCtx.newPage();
  await login(admin, "admin@controlhub.local", `${WORKSPACE}/actions`);
  const row = admin.getByRole("table", { name: "Action plans" }).getByRole("row").filter({ hasText: "proposed" }).filter({ hasText: "gateway" });
  await expect(row).toHaveCount(1);
  admin.once("dialog", (d) => void d.accept());
  await row.getByRole("button", { name: "Approve" }).click();

  await expect(detail.getByRole("heading", { level: 2 })).toContainText("succeeded", { timeout: 120_000 });
  await expect(detail.getByLabel("Verification")).toContainText('"passed": true');
  const trail = detail.getByLabel("Audit trail");
  for (const a of ["action.propose", "action.approve", "action.start", "action.finish"]) {
    await expect(trail).toContainText(a);
  }
  await opCtx.close();
  await adminCtx.close();
});

test("admin rejects an operator's plan with a required comment; the operator sees the decision", async ({ browser }) => {
  test.setTimeout(90_000);
  const reason = `e2e reject ${Date.now()}`;
  const opCtx = await browser.newContext();
  const op = await opCtx.newPage();
  await login(op, "operator@controlhub.local", `${WORKSPACE}/actions`);
  const form = op.getByRole("form", { name: "Propose action" });
  await form.getByLabel("Action").selectOption("demo.faults.clear");
  await form.getByLabel("Target").selectOption("orders");
  await form.getByLabel(/Reason/).fill(reason);
  await form.getByRole("button", { name: "Propose plan" }).click();
  const detail = op.getByRole("region", { name: "Plan detail" });
  const denied = op.locator("main").getByRole("alert");
  await expect(detail.getByRole("heading", { level: 2 }).or(denied)).toBeVisible();
  // The policy allows a limited number of plans per hour; other suites may have used them up.
  test.skip((await denied.count()) > 0 && /policy|limit|per hour|open plan/i.test((await denied.textContent()) ?? ""), "action policy quota reached");
  await expect(detail.getByRole("heading", { level: 2 })).toContainText("proposed");

  const adminCtx = await browser.newContext();
  const admin = await adminCtx.newPage();
  await login(admin, "admin@controlhub.local", `${WORKSPACE}/actions`);
  const row = admin.getByRole("table", { name: "Action plans" }).getByRole("row").filter({ hasText: "proposed" }).filter({ hasText: "orders" });
  await expect(row).toHaveCount(1);

  // A dismissed prompt changes nothing.
  admin.once("dialog", (d) => void d.dismiss());
  await row.getByRole("button", { name: "Reject" }).click();
  await expect(row).toContainText("proposed");

  admin.once("dialog", (d) => {
    expect(d.type()).toBe("prompt");
    void d.accept("not needed during the e2e run");
  });
  await row.getByRole("button", { name: "Reject" }).click();
  const decided = admin.getByRole("table", { name: "Action plans" }).getByRole("row").filter({ hasText: "rejected" }).filter({ hasText: "orders" }).first();
  await expect(decided).toBeVisible();
  await expect(decided.getByRole("button")).toHaveCount(0);

  await expect(detail.getByRole("heading", { level: 2 })).toContainText("rejected", { timeout: 30_000 });
  await op.reload();
  await op.getByRole("table", { name: "Action plans" }).getByRole("row").filter({ hasText: "rejected" }).filter({ hasText: "orders" }).first().click();
  await expect(detail).toContainText("Decision comment: not needed during the e2e run");
  await expect(detail.getByLabel("Audit trail")).toContainText("action.reject");
  await opCtx.close();
  await adminCtx.close();
});

test("viewer sees plans but cannot propose", async ({ page }) => {
  await login(page, "viewer@controlhub.local", `${WORKSPACE}/actions`);
  await expect(page.getByText("Proposing actions requires the operator role.")).toBeVisible();
  await expect(page.getByRole("table", { name: "Action plans" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Approve" })).toHaveCount(0);
});
