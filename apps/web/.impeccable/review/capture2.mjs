import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
const require = createRequire(new URL("../../../../tests/e2e/package.json", import.meta.url));
const { chromium } = require("@playwright/test");

const out = new URL(".", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const creds = JSON.parse(readFileSync(new URL("../../../../.karbird/test-credentials.json", import.meta.url), "utf8"));
const u = creds.users.find((x) => x.email.startsWith("isolation@")) ?? creds.users[0];

const browser = await chromium.launch();
for (const vp of [
  { name: "desktop", w: 1440, h: 900 },
  { name: "mobile", w: 390, h: 844 },
]) {
  const page = await browser.newPage({ viewport: { width: vp.w, height: vp.h }, reducedMotion: "reduce" });
  await page.goto("http://127.0.0.1:3000/app/login?next=%2Fapp%2Faccount");
  await page.getByLabel("Email").fill(u.email);
  await page.getByLabel("Password").fill(u.password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await page.waitForURL(/\/app\/account$/);
  await page.getByText(/^Welcome,/).waitFor();
  await page.screenshot({ path: `${out}account-${vp.name}.png`, fullPage: true });
  await page.goto("http://127.0.0.1:3000/app");
  await page.getByRole("heading", { level: 1, name: "Choose an environment" }).waitFor();
  await page.screenshot({ path: `${out}picker-${vp.name}.png`, fullPage: true });
  await page.close();
}
const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, reducedMotion: "reduce" });
await page.goto("http://127.0.0.1:3000/", { waitUntil: "networkidle" });
await page.screenshot({ path: `${out}desktop.png`, fullPage: true });
await browser.close();
console.log("captured as", u.email.replace(/@.*/, "@…"));
