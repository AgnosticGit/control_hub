import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, type Page } from "@playwright/test";

interface Creds {
  users: Array<{ email: string; password: string; project_id: string; role: string }>;
}

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const creds = JSON.parse(readFileSync(join(root, ".controlhub", "local-credentials.json"), "utf8")) as Creds;

export function user(email: string) {
  const u = creds.users.find((x) => x.email === email);
  if (!u) throw new Error(`no local credentials for ${email}; run just bootstrap`);
  return u;
}

export async function login(page: Page, email: string, next = "/app") {
  const u = user(email);
  await page.goto(`/app/login?next=${encodeURIComponent(next)}`);
  await page.getByLabel("Email").fill(u.email);
  await page.getByLabel("Password").fill(u.password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page).not.toHaveURL(/\/app\/login/);
}
