import { expect, test } from "./isolated-test";
import { login } from "./helpers";

const dockerId = "a".repeat(64);
const workspace = "/app/demo/local-demo";

test("a project manager connects and confirms one Docker container, then can revoke Control", async ({ page, request }) => {
  await login(page, "admin@karbird.local", `${workspace}/control`);
  await expect(page.getByRole("heading", { name: "Karbird Control" })).toBeVisible();
  await page.getByLabel("Installation name").fill(`e2e-control-${Date.now()}`);
  await page.getByRole("button", { name: "Create one-time enrollment code" }).click();
  const code = await page.getByLabel("One-time code").textContent();
  expect(code).toMatch(/^chce_/);
  const card = page.getByRole("article", { name: /^Control e2e-control-/ }).first();
  try {
    const enroll = await request.post("/api/v1/control/enroll", { data: { enrollment_code: code } });
    expect(enroll.status()).toBe(200);
    const { credential } = await enroll.json() as { credential: string };
    const report = await request.post("/api/v1/control/report", {
      headers: { authorization: `Bearer ${credential}` },
      data: { container_id: dockerId, container_name: "control-e2e", state: "running" },
    });
    expect(report.status()).toBe(200);
    await expect(card).toContainText("control-e2e");
    await card.getByLabel("Confirm the full Docker ID shown above").fill(dockerId);
    await card.getByRole("button", { name: "Allow control of this container" }).click();
    await expect(card).toContainText("Connected");
    await page.getByRole("link", { name: "Actions", exact: true }).click();
    await expect(page.getByRole("form", { name: "Run action" }).getByLabel("Action")).toContainText("Restart container");
  } finally {
    await page.goto(`${workspace}/control`);
    await page.getByRole("article", { name: /^Control e2e-control-/ }).first().getByRole("button", { name: "Revoke installation" }).click();
    await expect(page.getByRole("article", { name: /^Control e2e-control-/ }).first()).toContainText("Revoked");
  }
});
