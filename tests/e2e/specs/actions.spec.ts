import { expect, test } from "./isolated-test";
import { login } from "./helpers";

const WORKSPACE = "/app/demo/local-demo";

test("operator proposes, an admin approves, the executor runs and verifies", async ({ browser }) => {
  test.setTimeout(150_000);
  const reason = `e2e ${Date.now()}`;

  const opCtx = await browser.newContext();
  const op = await opCtx.newPage();
  await login(op, "operator@karbird.local", WORKSPACE);
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
  const opRow = op.getByRole("table", { name: "Action plans" }).getByRole("row").filter({ hasText: "operator@karbird.local" }).first();
  await expect(opRow.getByRole("button", { name: "Approve" })).toHaveCount(0);

  const adminCtx = await browser.newContext();
  const admin = await adminCtx.newPage();
  await login(admin, "admin@karbird.local", `${WORKSPACE}/actions`);
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
  await login(op, "operator@karbird.local", `${WORKSPACE}/actions`);
  const form = op.getByRole("form", { name: "Propose action" });
  await form.getByLabel("Action").selectOption("demo.faults.clear");
  await form.getByLabel("Target").selectOption("orders");
  await form.getByLabel(/Reason/).fill(reason);
  await form.getByRole("button", { name: "Propose plan" }).click();
  const detail = op.getByRole("region", { name: "Plan detail" });
  const denied = op.locator("main").getByRole("alert");
  await expect(detail.getByRole("heading", { level: 2 }).or(denied)).toBeVisible();
  // One open plan per target: an interrupted run may have left one behind.
  test.skip((await denied.count()) > 0 && /open plan/i.test((await denied.textContent()) ?? ""), "an open plan for this target is left over");
  await expect(detail.getByRole("heading", { level: 2 })).toContainText("proposed");

  const adminCtx = await browser.newContext();
  const admin = await adminCtx.newPage();
  await login(admin, "admin@karbird.local", `${WORKSPACE}/actions`);
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

test("a feature flag is switched on with the checkbox, approved, run and verified", async ({ browser }) => {
  test.setTimeout(150_000);
  const reason = `e2e flag ${Date.now()}`;
  const opCtx = await browser.newContext();
  const adminCtx = await browser.newContext();
  try {
    const op = await opCtx.newPage();
    await login(op, "operator@karbird.local", `${WORKSPACE}/actions`);
    const form = op.getByRole("form", { name: "Propose action" });
    await form.getByLabel("Action").selectOption("demo.feature_flag.set");
    await form.getByLabel("Target").selectOption("catalog-cache");
    const enabled = form.getByRole("checkbox", { name: "enabled" });
    await expect(enabled).not.toBeChecked();
    await enabled.check();
    await expect(form).toContainText("Verification: The flag reads back as requested");
    await form.getByLabel(/Reason/).fill(reason);
    await form.getByRole("button", { name: "Propose plan" }).click();

    const detail = op.getByRole("region", { name: "Plan detail" });
    const denied = op.locator("main").getByRole("alert");
    await expect(detail.getByRole("heading", { level: 2 }).or(denied)).toBeVisible();
    test.skip((await denied.count()) > 0 && /open plan/i.test((await denied.textContent()) ?? ""), "an open plan for this target is left over");
    await expect(detail.getByRole("heading", { level: 2 })).toContainText("Set feature flag · catalog-cache");

    const admin = await adminCtx.newPage();
    await login(admin, "admin@karbird.local", `${WORKSPACE}/actions`);
    const row = admin.getByRole("table", { name: "Action plans" }).getByRole("row").filter({ hasText: "proposed" }).filter({ hasText: "catalog-cache" });
    await expect(row).toHaveCount(1);
    admin.once("dialog", (d) => void d.accept());
    await row.getByRole("button", { name: "Approve" }).click();

    await expect(detail.getByRole("heading", { level: 2 })).toContainText("succeeded", { timeout: 120_000 });
    await expect(detail.getByLabel("Result")).toContainText('"catalog-cache": true');
    const verification = detail.getByLabel("Verification");
    await expect(verification).toContainText('"service": "gateway"');
    await expect(verification).toContainText('"passed": true');
  } finally {
    // Put the demo back for other suites; the flag only exists in the local demo gateway.
    await fetch("http://127.0.0.1:7001/__demo/flags", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ flag: "catalog-cache", enabled: false }),
    }).catch(() => undefined);
    await opCtx.close();
    await adminCtx.close();
  }
});

test("an admin runs an action from the Actions page without a second approval", async ({ page }) => {
  test.setTimeout(150_000);
  await login(page, "admin@karbird.local", `${WORKSPACE}/actions`);
  const form = page.getByRole("form", { name: "Run action" });
  await expect(form).toContainText("Your role may run actions");
  await form.getByLabel("Action").selectOption("demo.faults.clear");
  await form.getByLabel("Target").selectOption("orders");
  await form.getByLabel(/Reason/).fill(`e2e run ${Date.now()}`);
  await expect(form.getByRole("button", { name: "Propose plan" })).toHaveCount(0);
  await form.getByRole("button", { name: "Run" }).click();

  const detail = page.getByRole("region", { name: "Plan detail" });
  await expect(detail.getByRole("heading", { level: 2 })).toContainText(/approved|running|succeeded/);
  await expect(detail.getByRole("heading", { level: 2 })).toContainText("succeeded", { timeout: 120_000 });
  const trail = detail.getByLabel("Audit trail");
  for (const a of ["action.propose", "action.approve", "action.start", "action.finish"]) await expect(trail).toContainText(a);
});

test("viewer sees plans but cannot propose", async ({ page }) => {
  await login(page, "viewer@karbird.local", `${WORKSPACE}/actions`);
  await expect(page.getByText("Your role in this project does not include proposing actions.")).toBeVisible();
  await expect(page.getByRole("table", { name: "Action plans" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Approve" })).toHaveCount(0);
});
