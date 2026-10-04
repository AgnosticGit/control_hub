import { expect, test } from "@playwright/test";

test("home page is indexable and links to the app", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveTitle(/karbird/);
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
  await expect(page.getByRole("heading", { name: /One package, one start command/ })).toBeVisible();

  await page.goto("/getting-started");
  await expect(page.getByRole("heading", { name: "2. Create an ingestion token" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "3. Install the karbird SDK" })).toBeVisible();
  await expect(page.getByRole("main")).toContainText("KARBIRD_OTLP_ENDPOINT=https://<your-endpoint>/otlp");
});

test("public setup guides connect services only through the karbird SDK", async ({ page }) => {
  for (const path of ["/", "/getting-started", "/integrations"]) {
    await page.goto(path);
    const main = page.getByRole("main");
    await expect(main).toContainText("npm install @karbird/sdk-node");
    await expect(main).toContainText("node --import @karbird/sdk-node/register");
    await expect(main).not.toContainText(/OTEL_|@opentelemetry\/|Collector/);
  }
});

test("public header shows the karbird owl logo linking home", async ({ page }) => {
  await page.goto("/features");
  const home = page.getByRole("banner").getByRole("link", { name: "karbird", exact: true });
  await expect(home).toHaveAttribute("href", "/");
  const owl = home.locator("img");
  await expect(owl).toBeVisible();
  await expect.poll(() => owl.evaluate((img: HTMLImageElement) => img.complete && img.naturalWidth)).toBeGreaterThan(0);
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
