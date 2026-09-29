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

test("viewer sees plans but cannot propose", async ({ page }) => {
  await login(page, "viewer@controlhub.local", `${WORKSPACE}/actions`);
  await expect(page.getByText("Proposing actions requires the operator role.")).toBeVisible();
  await expect(page.getByRole("table", { name: "Action plans" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Approve" })).toHaveCount(0);
});
