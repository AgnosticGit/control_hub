import { expect, test } from "@playwright/test";

test("home page is indexable and links to the app", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveTitle(/ControlHub/);
  await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
  await expect(page.locator('meta[name="robots"]')).toHaveCount(0);
});

test("the public site sells the hosted service: sign-up and API setup, no local install", async ({ page }) => {
  for (const path of ["/", "/getting-started"]) {
    await page.goto(path);
    const main = page.getByRole("main");
    await expect(main).not.toContainText(/just (bootstrap|dev|demo)|run it locally|on your machine|localhost/i);
  }

  await page.goto("/");
  await expect(page.getByRole("link", { name: "Create account" })).toHaveAttribute("href", "/app/register");
  await expect(page.getByRole("heading", { name: /Connect over the API/ })).toBeVisible();

  await page.goto("/getting-started");
  await expect(page.getByRole("heading", { name: "2. Create an ingestion token" })).toBeVisible();
  await expect(page.getByRole("main")).toContainText("OTEL_EXPORTER_OTLP_ENDPOINT=https://<your-endpoint>/otlp");
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
