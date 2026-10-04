// Cuts the round owl-face badge out of karbird-face-site.jpg into a transparent PNG for the site header.
import { writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(join(here, "../../../tests/e2e/package.json"));
const { chromium } = require("@playwright/test");

const size = 256;
const output = join(here, "../../../apps/web/public/brand/karbird-owl.png");

const browser = await chromium.launch();
const page = await browser.newPage();
await page.goto(pathToFileURL(join(here, "karbird-face-site.jpg")).href);
const dataUrl = await page.evaluate(async (size) => {
  const img = document.querySelector("img");
  await img.decode();
  const W = img.naturalWidth;
  const H = img.naturalHeight;
  const src = document.createElement("canvas");
  src.width = W;
  src.height = H;
  const sctx = src.getContext("2d");
  sctx.drawImage(img, 0, 0);
  const d = sctx.getImageData(0, 0, W, H).data;

  // The cyan ring is the outer edge of the badge.
  let minX = W, minY = H, maxX = 0, maxY = 0;
  for (let y = 0; y < H; y++) {
    for (let x = 0; x < W; x++) {
      const i = (y * W + x) * 4;
      const [r, g, b] = [d[i], d[i + 1], d[i + 2]];
      if (g > 150 && b > 150 && r < 140 && g - r > 60) {
        minX = Math.min(minX, x); maxX = Math.max(maxX, x);
        minY = Math.min(minY, y); maxY = Math.max(maxY, y);
      }
    }
  }
  const cx = (minX + maxX) / 2;
  const cy = (minY + maxY) / 2;
  const r = Math.max(maxX - minX, maxY - minY) / 2;

  const out = document.createElement("canvas");
  out.width = size;
  out.height = size;
  const ctx = out.getContext("2d");
  ctx.imageSmoothingQuality = "high";
  ctx.beginPath();
  ctx.arc(size / 2, size / 2, size / 2, 0, Math.PI * 2);
  ctx.clip();
  ctx.drawImage(src, cx - r, cy - r, r * 2, r * 2, 0, 0, size, size);
  return out.toDataURL("image/png");
}, size);
await browser.close();
writeFileSync(output, Buffer.from(dataUrl.split(",")[1], "base64"));
console.log(`wrote ${output}`);
