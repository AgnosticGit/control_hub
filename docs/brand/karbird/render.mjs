import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(join(here, "../../../tests/e2e/package.json"));
const { chromium } = require("@playwright/test");

const [, , input = "sheet.html", output = "sheet.png"] = process.argv;
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1600, height: 900 }, deviceScaleFactor: 2 });
await page.goto(pathToFileURL(join(here, input)).href, { waitUntil: "networkidle" });
await page.screenshot({ path: join(here, output), fullPage: true });
await browser.close();
