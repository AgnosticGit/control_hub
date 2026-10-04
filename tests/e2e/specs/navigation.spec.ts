import { expect, test } from "@playwright/test";
import { collectErrors, login } from "./helpers";

const PUBLIC_PAGES = [
  { path: "/", nav: null, title: /karbird/, h1: /See what your system is doing/ },
  { path: "/features", nav: "Features", title: /· karbird$/, h1: null },
  { path: "/integrations", nav: "Integrations", title: /· karbird$/, h1: null },
  { path: "/getting-started", nav: "Getting started", title: /· karbird$/, h1: null },
  { path: "/docs", nav: "Docs", title: /Documentation · karbird/, h1: /Documentation/ },
];

test.describe("public site", () => {
  for (const p of PUBLIC_PAGES) {
    test(`${p.path} renders with a title, one h1, a description and no errors`, async ({ page }) => {
      const errors = collectErrors(page);
      const res = await page.goto(p.path);
      expect(res?.status()).toBe(200);
      await expect(page).toHaveTitle(p.title);
      const h1 = page.getByRole("heading", { level: 1 });
      await expect(h1).toHaveCount(1);
      if (p.h1) await expect(h1).toHaveText(p.h1);
      await expect(page.locator('meta[name="description"]')).toHaveAttribute("content", /(?:\S\s*){10,}/);
      await expect(page.getByRole("navigation", { name: "Main" })).toBeVisible();
      await expect(page.getByRole("contentinfo")).toContainText("early development build");
      expect(errors).toEqual([]);
    });
  }

  test("main navigation links reach every public page and the app", async ({ page }) => {
    await page.goto("/");
    const nav = page.getByRole("navigation", { name: "Main" });
    for (const p of PUBLIC_PAGES.filter((x) => x.nav)) {
      await nav.getByRole("link", { name: p.nav ?? "", exact: true }).click();
      await expect(page).toHaveURL(new RegExp(`${p.path}$`));
      await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
    }
    await page.getByRole("link", { name: "karbird" }).first().click();
    await expect(page).toHaveURL(/\/$/);
    await expect(page.getByRole("link", { name: "Run it locally" })).toHaveCount(0);
    await page.getByRole("link", { name: "Create account" }).click();
    await expect(page).toHaveURL(/\/app\/register$/);
    await page.goto("/");
    await page.getByRole("link", { name: "How it works" }).click();
    await expect(page).toHaveURL(/\/features$/);
    await nav.getByRole("link", { name: "Open app" }).click();
    await expect(page).toHaveURL(/\/app\/login/);
  });

  test("every internal link on the public pages resolves (no 404s)", async ({ page, request }) => {
    const hrefs = new Set<string>();
    for (const p of PUBLIC_PAGES) {
      await page.goto(p.path);
      for (const href of await page.locator("a[href]").evaluateAll((as) => as.map((a) => a.getAttribute("href") ?? ""))) {
        if (href.startsWith("/") && !href.startsWith("//")) hrefs.add(href.split("#")[0] ?? href);
      }
    }
    expect(hrefs.size).toBeGreaterThanOrEqual(6);
    for (const href of hrefs) {
      const res = await request.get(href, { maxRedirects: 0 });
      expect([200, 307, 308], `${href} -> ${res.status()}`).toContain(res.status());
    }
  });

  test("sitemap lists the public pages only", async ({ request }) => {
    const res = await request.get("/sitemap.xml");
    expect(res.ok()).toBeTruthy();
    const xml = await res.text();
    for (const p of ["/features", "/integrations", "/getting-started", "/docs"]) expect(xml).toContain(`${p}</loc>`);
    expect(xml).not.toContain("/app");
  });

  test("unknown public pages return 404", async ({ page }) => {
    const res = await page.goto("/definitely-not-a-page");
    expect(res?.status()).toBe(404);
  });
});

test.describe("app navigation", () => {
  test("a user with several environments picks one; demo environments are labelled", async ({ page }) => {
    await login(page, "isolation@karbird.local", "/app");
    await expect(page).toHaveURL(/\/app$/);
    await expect(page.getByRole("heading", { level: 1, name: "Choose an environment" })).toBeVisible();
    await expect(page.getByText("Isolation", { exact: false }).first()).toBeVisible();
    await expect(page.getByText("Owner", { exact: true }), "the role name is shown per project").toBeVisible();
    const prod = page.getByRole("link", { name: /isolation-prod/ });
    const testEnv = page.getByRole("link", { name: /isolation-test/ });
    await expect(prod).toBeVisible();
    await expect(testEnv).toContainText("DEMO");
    await expect(prod).not.toContainText("DEMO");
    await expect(page.getByRole("link", { name: /local-demo/ })).toHaveCount(0);

    await prod.click();
    await expect(page).toHaveURL(/\/app\/isolation\/isolation-prod$/);
    const sidebar = page.getByRole("complementary", { name: "Sidebar" });
    await expect(sidebar).toContainText("isolation-prod");
    await expect(sidebar.getByText("DEMO", { exact: true })).toHaveCount(0);

    await sidebar.getByRole("link", { name: "Switch environment" }).click();
    await expect(page).toHaveURL(/\/app$/);
    await page.getByRole("link", { name: /isolation-test/ }).click();
    await expect(page).toHaveURL(/\/app\/isolation\/isolation-test$/);
    await expect(page.getByRole("complementary", { name: "Sidebar" }).getByText("DEMO", { exact: true })).toBeVisible();
  });

  test("a single-environment user is forwarded from /app into the workspace", async ({ page }) => {
    await login(page, "viewer@karbird.local", "/app");
    await expect(page).toHaveURL(/\/app\/demo\/local-demo$/);
    await page.getByRole("complementary", { name: "Sidebar" }).getByRole("link", { name: "Switch environment" }).click();
    await expect(page).toHaveURL(/\/app\/demo\/local-demo$/);
  });

  test("sidebar navigation marks the current page and reaches every section", async ({ page }) => {
    await login(page, "admin@karbird.local", "/app/demo/local-demo");
    const nav = page.getByRole("navigation", { name: "Workspace" });
    await expect(nav.getByRole("link", { name: "Overview" })).toHaveAttribute("aria-current", "page");
    await nav.getByRole("link", { name: "Actions" }).click();
    await expect(page).toHaveURL(/\/actions$/);
    await expect(nav.getByRole("link", { name: "Actions" })).toHaveAttribute("aria-current", "page");
    await expect(nav.getByRole("link", { name: "Overview" })).not.toHaveAttribute("aria-current", "page");
    await nav.getByRole("link", { name: "Tokens" }).click();
    await expect(page).toHaveURL(/\/tokens$/);
    await expect(nav.getByRole("link", { name: "Tokens" })).toHaveAttribute("aria-current", "page");
    await nav.getByRole("link", { name: "Overview" }).click();
    await expect(page).toHaveURL(/\/app\/demo\/local-demo$/);
    await expect(page.getByRole("complementary", { name: "Sidebar" })).toContainText("admin@karbird.local");
  });

  for (const path of ["/app/demo/no-such-env", "/app/no-such-project/local-demo", "/app/isolation/isolation-test"]) {
    test(`${path} (overview, actions, tokens) shows a not-found message without leaking data`, async ({ page }) => {
      await login(page, "admin@karbird.local", "/app");
      for (const section of ["", "/actions", "/tokens"]) {
        await page.goto(`${path}${section}`);
        await expect(page.getByText("This environment does not exist or you do not have access to it.")).toBeVisible();
        await expect(page.getByRole("table")).toHaveCount(0);
        await expect(page.getByRole("region", { name: "Signals" })).toHaveCount(0);
      }
      await page.getByRole("link", { name: "Back to environments" }).click();
      await expect(page).toHaveURL(/\/app\/demo\/local-demo$/);
    });
  }

  test("unknown routes inside the app return 404", async ({ page }) => {
    await login(page, "admin@karbird.local", "/app");
    const res = await page.goto("/app/demo/local-demo/does-not-exist");
    expect(res?.status()).toBe(404);
  });
});
