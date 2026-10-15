import { expect, test, type Page } from "./isolated-test";
import { login } from "./helpers";

/** Structural accessibility problems a screen reader user would hit on the current page. */
async function audit(page: Page) {
  return page.evaluate(() => {
    const visible = (el: Element) => {
      const r = el.getBoundingClientRect();
      const s = getComputedStyle(el);
      return r.width > 0 && r.height > 0 && s.visibility !== "hidden" && s.display !== "none";
    };
    const name = (el: Element) =>
      (el.getAttribute("aria-label") ?? "").trim() ||
      (el.getAttribute("aria-labelledby") ? "labelled" : "") ||
      (el.getAttribute("title") ?? "").trim() ||
      (el.textContent ?? "").trim();
    const ids = [...document.querySelectorAll("[id]")].map((e) => e.id).filter(Boolean);
    const duplicateIds = [...new Set(ids.filter((id, i) => ids.indexOf(id) !== i))];
    const unnamedButtons = [...document.querySelectorAll("button, [role=button], a[href]")]
      .filter(visible)
      .filter((b) => !name(b))
      .map((b) => b.outerHTML.slice(0, 120));
    const unlabelledInputs = [...document.querySelectorAll("input:not([type=hidden]), select, textarea")]
      .filter(visible)
      .filter((i) => {
        const el = i as HTMLInputElement;
        return !el.labels?.length && !el.getAttribute("aria-label") && !el.getAttribute("aria-labelledby") && !el.getAttribute("title");
      })
      .map((i) => i.outerHTML.slice(0, 120));
    return {
      lang: document.documentElement.lang,
      title: document.title,
      mains: document.querySelectorAll("main, [role=main]").length,
      h1s: [...document.querySelectorAll("h1")].filter(visible).map((h) => (h.textContent ?? "").trim()),
      duplicateIds,
      unnamedButtons,
      unlabelledInputs,
    };
  });
}

const PUBLIC = ["/", "/features", "/integrations", "/getting-started", "/docs", "/app/login", "/app/register"];

for (const path of PUBLIC) {
  test(`${path}: lang, title, one main landmark, one h1, named controls, unique IDs`, async ({ page }) => {
    await page.goto(path);
    await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
    const a = await audit(page);
    expect(a.lang).toBe("en");
    expect(a.title.length).toBeGreaterThan(5);
    expect(a.mains).toBe(1);
    expect(a.h1s).toHaveLength(1);
    expect(a.duplicateIds).toEqual([]);
    expect(a.unnamedButtons).toEqual([]);
    expect(a.unlabelledInputs).toEqual([]);
  });
}

test.describe("signed-in pages", () => {
  test.beforeEach(async ({ page }) => {
    await login(page, "admin@karbird.local", "/app/demo/local-demo");
    await expect(page.getByText("Starting engine…")).toBeHidden();
  });

  test("workspace: one main landmark, named controls, unique IDs, labelled regions", async ({ page }) => {
    await expect(page.getByRole("main", { name: "System map" })).toBeVisible();
    const a = await audit(page);
    expect(a.lang).toBe("en");
    expect(a.mains).toBe(1);
    expect(a.duplicateIds).toEqual([]);
    expect(a.unnamedButtons).toEqual([]);
    expect(a.unlabelledInputs).toEqual([]);
    await expect(page.getByRole("complementary", { name: "Sidebar" })).toBeVisible();
    await expect(page.getByRole("region", { name: "Timeline" })).toBeVisible();
    await expect(page.getByRole("region", { name: "Signals" })).toBeVisible();
    await expect(page.getByRole("navigation", { name: "Workspace" })).toBeVisible();
  });

  // The page heading remains available to screen readers without restoring the map badge.
  test("workspace has exactly one h1", async ({ page }) => {
    await expect(page.getByRole("heading", { level: 1, name: "System Map" })).toBeVisible();
    const a = await audit(page);
    expect(a.h1s).toEqual(["System Map"]);
  });

  for (const section of ["actions", "tokens", "integration"]) {
    test(`${section}: one main landmark, one h1, named controls, unique IDs`, async ({ page }) => {
      await page.goto(`/app/demo/local-demo/${section}`);
      await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
      const a = await audit(page);
      expect(a.mains).toBe(1);
      expect(a.h1s).toHaveLength(1);
      expect(a.duplicateIds).toEqual([]);
      expect(a.unnamedButtons).toEqual([]);
      expect(a.unlabelledInputs).toEqual([]);
    });
  }

  test("keyboard: the map, ruler and splitters are focusable with visible names", async ({ page }) => {
    for (const locator of [
      page.getByRole("application", { name: /Service map/ }),
      page.getByRole("slider", { name: "Selected time" }),
      page.getByRole("separator", { name: "Resize side panel" }),
      page.getByRole("separator", { name: "Resize bottom dock" }),
    ]) {
      await locator.focus();
      await expect(locator).toBeFocused();
    }
  });
});

test("app index (environment picker): one main landmark and one h1", async ({ page }) => {
  await login(page, "isolation@karbird.local", "/app");
  await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
  const a = await audit(page);
  expect(a.mains).toBe(1);
  expect(a.h1s).toEqual(["Choose an environment"]);
  expect(a.duplicateIds).toEqual([]);
  expect(a.unnamedButtons).toEqual([]);
});
