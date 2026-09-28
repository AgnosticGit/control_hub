import { expect, test } from "@playwright/test";

test("home page is indexable and links to the app", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveTitle(/ControlHub/);
  await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
  await expect(page.locator('meta[name="robots"]')).toHaveCount(0);
});

test("robots.txt disallows the private app", async ({ request }) => {
  const res = await request.get("/robots.txt");
  expect(res.ok()).toBeTruthy();
  expect(await res.text()).toContain("Disallow: /app");
});

test("private app redirects to login without a session and is noindex", async ({ page }) => {
  const res = await page.goto("/app/demo/local-demo");
  await expect(page).toHaveURL(/\/app\/login\?next=/);
  expect(res?.headers()["x-robots-tag"]).toContain("noindex");
});
