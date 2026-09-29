import { expect, test } from "@playwright/test";
import { login, user } from "./helpers";

const WORKSPACE = "/app/demo/local-demo";

test.describe("login page", () => {
  test("renders a labelled form and rejects wrong credentials with a generic message", async ({ page }) => {
    await page.goto("/app/login");
    await expect(page.getByRole("heading", { level: 1, name: "Sign in to ControlHub" })).toBeVisible();
    await expect(page.getByLabel("Email")).toHaveAttribute("type", "email");
    await expect(page.getByLabel("Password")).toHaveAttribute("type", "password");
    await expect(page.getByLabel("Password")).toHaveAttribute("autocomplete", "current-password");

    // A never-used address: the per-account throttle must not lock out seeded users.
    await page.getByLabel("Email").fill(`nobody-${Date.now()}@controlhub.local`);
    await page.getByLabel("Password").fill("definitely-wrong");
    await page.getByRole("button", { name: "Sign in" }).click();
    await expect(page.locator("form").getByRole("alert")).toHaveText("Invalid email or password.");
    await expect(page).toHaveURL(/\/app\/login/);
    const cookies = await page.context().cookies();
    expect(cookies.find((c) => c.name === "controlhub_session")).toBeUndefined();
  });

  test("an existing account with a wrong password gets the same message", async ({ page }) => {
    await page.goto("/app/login");
    await page.getByLabel("Email").fill(user("viewer@controlhub.local").email);
    await page.getByLabel("Password").fill("wrong-password");
    await page.getByRole("button", { name: "Sign in" }).click();
    await expect(page.locator("form").getByRole("alert")).toHaveText("Invalid email or password.");
    // A successful login resets the failure counter for this account.
    await login(page, "viewer@controlhub.local", WORKSPACE);
  });

  test("required fields are enforced by the browser before any request", async ({ page }) => {
    let posted = false;
    page.on("request", (r) => {
      if (r.url().includes("/api/v1/session/login")) posted = true;
    });
    await page.goto("/app/login");
    await page.getByRole("button", { name: "Sign in" }).click();
    await expect(page.getByLabel("Email")).toHaveJSProperty("validity.valueMissing", true);
    expect(posted).toBe(false);
  });

  test("a same-app `next` path is honoured after login", async ({ page }) => {
    await login(page, "admin@controlhub.local", `${WORKSPACE}/actions`);
    await expect(page).toHaveURL(new RegExp(`${WORKSPACE}/actions$`));
    await expect(page.getByRole("heading", { level: 1, name: "Actions" })).toBeVisible();
  });

  for (const next of ["https://evil.example/", "//evil.example/app", "/\\evil.example", "javascript:alert(1)", "/features"]) {
    test(`an unsafe or non-app next (${next}) falls back to /app on the same origin`, async ({ page, baseURL }) => {
      await login(page, "admin@controlhub.local", next);
      const url = new URL(page.url());
      expect(url.origin).toBe(new URL(baseURL ?? "http://127.0.0.1:3000").origin);
      // /app forwards a user with a single environment straight into it.
      await expect(page).toHaveURL(new RegExp(`${WORKSPACE}$`));
    });
  }
});

test.describe("session lifecycle", () => {
  test("private pages redirect to login and come back to the requested page", async ({ page }) => {
    await page.goto(`${WORKSPACE}/tokens`);
    await expect(page).toHaveURL(/\/app\/login\?next=%2Fapp%2Fdemo%2Flocal-demo%2Ftokens/);
    const u = user("admin@controlhub.local");
    await page.getByLabel("Email").fill(u.email);
    await page.getByLabel("Password").fill(u.password);
    await page.getByRole("button", { name: "Sign in" }).click();
    await expect(page).toHaveURL(new RegExp(`${WORKSPACE}/tokens$`));
    await expect(page.getByRole("heading", { level: 1, name: "Tokens" })).toBeVisible();
  });

  test("a stale session cookie is detected by the API and sends the user to login", async ({ page, baseURL }) => {
    const origin = new URL(baseURL ?? "http://127.0.0.1:3000");
    await page.context().addCookies([{ name: "controlhub_session", value: "chs_stale", domain: origin.hostname, path: "/" }]);
    await page.goto(`${WORKSPACE}/actions`);
    await expect(page).toHaveURL(/\/app\/login\?next=%2Fapp%2Fdemo%2Flocal-demo%2Factions/);
  });

  test("sign out clears the session: private pages and the API reject the old cookie", async ({ page }) => {
    await login(page, "viewer@controlhub.local", WORKSPACE);
    await expect(page.getByText("DEMO", { exact: true })).toBeVisible();
    const before = await page.context().cookies();
    const session = before.find((c) => c.name === "controlhub_session");
    expect(session?.httpOnly).toBe(true);
    expect(session?.sameSite).toBe("Strict");
    expect(before.find((c) => c.name === "controlhub_csrf")?.httpOnly).toBe(false);

    await page.getByRole("button", { name: "Sign out" }).click();
    await expect(page).toHaveURL(/\/app\/login$/);
    const after = await page.context().cookies();
    expect(after.find((c) => c.name === "controlhub_session")).toBeUndefined();

    await page.goto(WORKSPACE);
    await expect(page).toHaveURL(/\/app\/login\?next=/);
    // The server revoked the session: replaying the captured cookie does not work either.
    const replay = await page.request.get("/api/v1/session", { headers: { cookie: `controlhub_session=${session?.value ?? ""}` } });
    expect(replay.status()).toBe(401);
  });
});
