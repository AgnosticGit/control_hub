import { expect, test } from "@playwright/test";
import { collectErrors, ingestSpans, login, randomHex } from "./helpers";

const WORKSPACE = "/app/isolation/isolation-test";

test("requests from outside the project and calls to third-party services are marked as external", async ({ page }) => {
  const run = randomHex(4);
  const front = `front-${run}`;
  const extHost = `api.ext-${run}.test`;
  const traceId = randomHex(16);
  const [rootId, callId] = [randomHex(8), randomHex(8)];
  const t = BigInt(Date.now() - 3_000) * 1_000_000n;
  const ms = 1_000_000n;
  await ingestSpans(front, [
    { traceId, spanId: rootId, name: "GET /checkout", kind: 2, startNs: t, endNs: t + 40n * ms, attributes: { "http.request.method": "GET" } },
    {
      traceId,
      spanId: callId,
      parentSpanId: rootId,
      name: "GET",
      kind: 3,
      startNs: t + 5n * ms,
      endNs: t + 25n * ms,
      attributes: { "http.request.method": "GET", "server.address": extHost },
    },
  ]);

  const errors = collectErrors(page);
  await login(page, "isolation@controlhub.local", WORKSPACE);
  await expect(page.getByText("Starting engine…")).toBeHidden();
  await page.getByRole("checkbox", { name: "Table view" }).check();
  const table = page.getByRole("table");
  const row = (name: string) => table.getByRole("row").filter({ has: page.getByRole("button", { name, exact: true }) });

  // The topology is re-read periodically; reload until the new service has arrived.
  await expect
    .poll(
      async () => {
        if ((await row(extHost).count()) > 0) return true;
        await page.reload();
        await page.getByRole("checkbox", { name: "Table view" }).check();
        return false;
      },
      { timeout: 45_000, intervals: [2_000] },
    )
    .toBe(true);

  await expect(row(extHost)).toContainText("External service");
  await expect(row("External clients")).toContainText("External client");
  await expect(row(front)).toContainText("service");
  await expect(row(front)).not.toContainText("External");

  await row(extHost).getByRole("button", { name: extHost, exact: true }).click();
  const inspector = page.getByRole("complementary", { name: "Inspector" });
  await expect(inspector.getByRole("heading", { name: extHost, exact: true })).toBeVisible();
  await expect(inspector.getByText("External service", { exact: true })).toBeVisible();
  await expect(inspector).toContainText("Not part of this project");
  const relations = inspector.getByRole("region", { name: "Relations" });
  await expect(relations.getByRole("button", { name: front, exact: true })).toBeVisible();

  await relations.getByRole("button", { name: front, exact: true }).click();
  await expect(inspector.getByRole("heading", { name: front, exact: true })).toBeVisible();
  await expect(relations.getByRole("button", { name: "External clients", exact: true })).toBeVisible();
  await relations.getByRole("button", { name: "External clients", exact: true }).click();
  await expect(inspector.getByText("External client", { exact: true })).toBeVisible();
  await expect(inspector).toContainText("requests that arrived without an instrumented caller");

  expect(errors).toEqual([]);
});
