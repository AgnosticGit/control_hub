import { randomUUID } from "node:crypto";
import { expect, test, type Page } from "@playwright/test";
import { ingestLog, login, randomHex } from "./helpers";

const PASSWORD = `correct horse ${randomHex(4)}`;
const newEmail = () => `e2e-${randomUUID()}@example.test`;

async function fillSignup(page: Page, email: string, password = PASSWORD, confirm = password) {
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password", { exact: true }).fill(password);
  await page.getByLabel("Repeat password").fill(confirm);
}

test("a visitor signs up, gets their own project and connects a service from the integration guide", async ({ page, baseURL }) => {
  const origin = new URL(baseURL ?? "http://127.0.0.1:3000").origin;
  const email = newEmail();
  const project = `E2E shop ${randomHex(3)}`;
  const urls: string[] = [];
  page.on("framenavigated", (f) => urls.push(f.url()));

  await page.goto("/app/login");
  await page.getByRole("link", { name: "Create an account" }).click();
  await expect(page).toHaveURL(/\/app\/register$/);
  await expect(page.getByRole("heading", { level: 1, name: "Create your ControlHub account" })).toBeVisible();
  await fillSignup(page, email);
  await page.getByLabel("Project name (optional)").fill(project);
  await page.getByRole("button", { name: "Create account" }).click();

  await expect(page).toHaveURL(/\/app\/p-[0-9a-f]{32}\/e-[0-9a-f]{32}\/integration$/);
  await expect(page.getByRole("heading", { level: 1, name: "Connect your services" })).toBeVisible();
  const sidebar = page.getByRole("complementary", { name: "Sidebar" });
  await expect(sidebar.getByText(project)).toBeVisible();
  await expect(sidebar.getByText("production")).toBeVisible();
  await expect(sidebar.getByText("DEMO", { exact: true })).toHaveCount(0);
  await expect(page.getByRole("navigation", { name: "Workspace" }).getByRole("link", { name: "Integration" })).toHaveAttribute(
    "aria-current",
    "page",
  );
  const cookies = await page.context().cookies();
  expect(cookies.find((c) => c.name === "controlhub_session")?.httpOnly).toBe(true);
  expect(urls.filter((u) => u.includes(encodeURIComponent(PASSWORD)) || u.includes("password"))).toEqual([]);

  const endpoint = `${origin}/otlp`;
  await expect(page.getByLabel("OTLP endpoint")).toHaveText(endpoint);
  const env = page.getByLabel("OpenTelemetry environment variables", { exact: true });
  await expect(env).toContainText("<your-ingestion-token>");
  await expect(page.getByRole("status", { name: "Telemetry status" })).toContainText("Waiting for the first telemetry");

  const service = `e2e-signup-${randomHex(3)}`;
  await page.getByLabel("Service name").fill(service);
  await page.getByRole("button", { name: "Create ingestion token" }).click();
  const secret = page.getByLabel("New ingestion token");
  await expect(secret).toHaveText(/^chi_[A-Za-z0-9_-]+$/);
  const token = (await secret.textContent()) ?? "";
  await expect(env).toContainText(`OTEL_EXPORTER_OTLP_HEADERS=authorization=Bearer%20${token}`);
  await expect(env).toContainText(`OTEL_EXPORTER_OTLP_ENDPOINT=${endpoint}`);
  await expect(env).toContainText(`OTEL_SERVICE_NAME=${service}`);
  await expect(page.getByLabel("Collector configuration", { exact: true })).toContainText(token);

  // Send telemetry exactly as the guide says: through the site's /otlp address with the new token.
  await ingestLog({ service, body: "hello from a new customer", token, endpoint });
  await expect(page.getByRole("status", { name: "Telemetry status" })).toContainText(`Receiving data from 1 service: ${service}`, {
    timeout: 30_000,
  });

  // The secret is shown once: it is gone after a reload, and the token is listed by name only.
  await page.reload();
  await expect(page.getByLabel("New ingestion token")).toHaveCount(0);
  await page.getByRole("navigation", { name: "Workspace" }).getByRole("link", { name: "Tokens" }).click();
  const tokens = page.getByRole("table", { name: "Tokens" });
  await expect(tokens).toContainText(`integration ${service}`);
  await expect(tokens).not.toContainText(token);
});

test("a new account can sign out and sign back in with its password", async ({ page }) => {
  const email = newEmail();
  await page.goto("/app/register");
  await fillSignup(page, email);
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Connect your services" })).toBeVisible();

  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page).toHaveURL(/\/app\/login$/);
  await page.getByLabel("Email").fill(email);
  await page.getByLabel("Password").fill(PASSWORD);
  await page.getByRole("button", { name: "Sign in" }).click();
  // A single environment opens straight away; the demo is not part of a new account.
  await expect(page).toHaveURL(/\/app\/p-[0-9a-f]{32}\/e-[0-9a-f]{32}$/);
  await page.goto("/app/demo/local-demo");
  await expect(page.getByText("This environment does not exist or you do not have access to it.")).toBeVisible();
});

test("sign-up explains problems and never creates a session for them", async ({ page }) => {
  await page.goto("/app/register");
  await fillSignup(page, newEmail(), PASSWORD, `${PASSWORD}!`);
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page.locator("form").getByRole("alert")).toHaveText("Passwords do not match.");

  await fillSignup(page, newEmail(), "short");
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page.locator("form").getByRole("alert")).toHaveText("Password must be at least 12 characters.");

  // An address that already has an account (a seeded demo user).
  await fillSignup(page, "Viewer@ControlHub.local");
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page.locator("form").getByRole("alert")).toHaveText("An account with this email already exists. Sign in instead.");
  await expect(page).toHaveURL(/\/app\/register$/);
  const cookies = await page.context().cookies();
  expect(cookies.find((c) => c.name === "controlhub_session")).toBeUndefined();

  // The seeded account is untouched.
  await login(page, "viewer@controlhub.local", "/app/demo/local-demo");
});
