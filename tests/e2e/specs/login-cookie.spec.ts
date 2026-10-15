import { expect, test } from "./isolated-test";

test("explains when login succeeds but the browser does not retain its session cookie", async ({ page }) => {
  await page.route("**/api/v1/session/login", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ user: {}, projects: [] }) }),
  );
  await page.route("**/api/v1/session", (route) =>
    route.fulfill({
      status: 401,
      contentType: "application/json",
      body: JSON.stringify({ code: "unauthenticated", message: "no session" }),
    }),
  );
  await page.goto("/app/login");
  await page.getByLabel("Email").fill("admin@example.test");
  await page.getByLabel("Password").fill("not-a-real-password");
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.locator("form").getByRole("alert")).toHaveText(
    "Login succeeded, but the browser did not keep the session. Open karbird over HTTPS and try again.",
  );
  await expect(page).toHaveURL(/\/app\/login$/);
});

test("reloads into the private app after the session cookie is verified", async ({ page }) => {
  const session = {
    organizations: [],
    projects: [],
    user: {
      created_at: "2026-01-01T00:00:00Z",
      display_name: "Admin",
      email: "admin@example.test",
      id: "owner-id",
      onboarding_done: true,
    },
    system: null,
  };
  await page.route("**/api/v1/session/login", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      headers: { "set-cookie": "karbird_session=verified; Path=/; HttpOnly; SameSite=Strict" },
      body: JSON.stringify(session),
    }),
  );
  await page.route("**/api/v1/session", (route) => route.fulfill({ status: 200, json: session }));
  await page.goto("/app/login");
  await page.getByLabel("Email").fill(session.user.email);
  await page.getByLabel("Password").fill("not-a-real-password");
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page).toHaveURL(/\/app$/);
  await expect(page.getByRole("heading", { name: "Choose an environment" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Create one" })).toHaveAttribute("href", "/app/account/projects");
});
