import { createRequire } from "node:module";
const require = createRequire(new URL("../../../../tests/e2e/package.json", import.meta.url));
const { chromium } = require("@playwright/test");

const out = new URL(".", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
const shots = [
  { path: "/", name: "desktop", w: 1440, h: 900, full: true },
  { path: "/", name: "mobile", w: 390, h: 844, full: true },
  { path: "/app/login", name: "login-desktop", w: 1440, h: 900, full: false },
  { path: "/app/register", name: "register-mobile", w: 390, h: 844, full: true },
];

const browser = await chromium.launch();
for (const s of shots) {
  const page = await browser.newPage({ viewport: { width: s.w, height: s.h }, reducedMotion: "reduce" });
  await page.goto(`http://127.0.0.1:3000${s.path}`, { waitUntil: "networkidle" });
  await page.waitForLoadState("networkidle");
  await page.screenshot({ path: `${out}${s.name}.png`, fullPage: s.full });
  await page.close();
}
const demo = await browser.newPage({ viewport: { width: 390, height: 844 }, reducedMotion: "reduce", deviceScaleFactor: 2 });
await demo.goto("http://127.0.0.1:3000/", { waitUntil: "networkidle" });
await demo.getByRole("group", { name: "Interactive demo" }).screenshot({ path: `${out}demo-mobile.png` });
await demo.close();
await browser.close();
console.log("captured", shots.map((s) => s.name).join(", "));
