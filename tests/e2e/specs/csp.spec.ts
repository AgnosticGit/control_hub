import { expect, test, type Page } from "@playwright/test";
import { login } from "./helpers";

function collectViolations(page: Page): string[] {
  const violations: string[] = [];
  page.on("console", (m) => {
    if (/Content Security Policy|Refused to (execute|load|connect|compile|create)/i.test(m.text())) violations.push(m.text());
  });
  return violations;
}

// `next dev` (used by `just dev`) needs 'unsafe-eval' for React's dev tooling, so the policy adds
// it outside production (see apps/web/src/lib/csp.ts). The no-eval guarantee is a production one.
async function isNextDev(page: Page): Promise<boolean> {
  return page
    .locator("nextjs-portal")
    .first()
    .waitFor({ state: "attached", timeout: 5_000 })
    .then(
      () => true,
      () => false,
    );
}

function expectEval(csp: string | undefined, dev: boolean): void {
  if (dev) expect(csp).toMatch(/script-src[^;]*'unsafe-eval'/);
  else expect(csp).not.toContain("'unsafe-eval'");
}

test("public pages send a CSP and run without violations", async ({ page }) => {
  const violations = collectViolations(page);
  const res = await page.goto("/");
  const csp = res?.headers()["content-security-policy"] ?? "";
  expect(csp).toContain("object-src 'none'");
  expect(csp).toContain("frame-ancestors 'none'");
  expectEval(csp, await isNextDev(page));
  await page.goto("/features");
  expect(violations).toEqual([]);
});

test("the workspace runs under a strict nonce CSP: map, WASM worker and WebSocket work", async ({ page }) => {
  const violations = collectViolations(page);
  await login(page, "admin@controlhub.local", "/app");
  // Full document load (login navigates client-side).
  const res = await page.goto("/app/demo/local-demo");
  const headers = (await res?.allHeaders()) ?? {};
  await expect(page.getByLabel("Live transport")).toHaveText("Live: WebSocket", { timeout: 30_000 });
  await expect(page.locator("canvas").first()).toBeVisible();
  const dev = await isNextDev(page);
  await page.getByRole("link", { name: "Actions" }).click();
  await expect(page.getByRole("heading", { name: /Actions/ })).toBeVisible();

  // Both policies arrive (joined by ", "); the strict one carries the nonce.
  const strict = (headers["content-security-policy"] ?? "").split(/,\s*(?=default-src)/).find((h) => h.includes("'nonce-"));
  expect(strict, headers["content-security-policy"]).toBeTruthy();
  expect(strict).toContain("'strict-dynamic'");
  expectEval(strict, dev);
  expect(strict).not.toMatch(/script-src[^;]*'unsafe-inline'/);
  expect(violations).toEqual([]);
});
