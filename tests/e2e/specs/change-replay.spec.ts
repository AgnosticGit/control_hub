import { expect, test } from "@playwright/test";
import { ingestEvent, ingestSpans, login, randomHex } from "./helpers";

test("a deployment can be selected and its before/after telemetry compared", async ({ page }) => {
  const service = `release-${randomHex(5)}`;
  const at = BigInt(Date.now()) * 1_000_000n - 5n * 60n * 1_000_000_000n;
  await ingestSpans(service, [
    { traceId: randomHex(16), spanId: randomHex(8), name: "before", kind: 2, startNs: at - 60n * 1_000_000_000n, endNs: at - 60n * 1_000_000_000n + 1_000_000n },
    { traceId: randomHex(16), spanId: randomHex(8), name: "after", kind: 2, startNs: at + 60n * 1_000_000_000n, endNs: at + 60n * 1_000_000_000n + 5_000_000n },
  ]);
  const startedId = await ingestEvent(service, "deployment.started", at, { "service.version": "2.0.0" });
  const finishedId = await ingestEvent(service, "deployment.finished", at, { "service.version": "2.0.0" });

  await login(page, "isolation@karbird.local", "/app/isolation/isolation-test/changes");
  const eventsUrl = `/api/v1/projects/isolation/environments/isolation-test/events?from=${at - 60n * 1_000_000_000n}&to=${at + 60n * 1_000_000_000n}&limit=20`;
  // The ingest response confirms Kafka accepted the events; wait for the processor's
  // ClickHouse write before asking the change page to refresh its one-shot query.
  await expect.poll(async () => {
    const response = await page.request.get(eventsUrl);
    if (!response.ok()) return false;
    const result = await response.json() as { events: Array<{ id: string }> };
    const ids = new Set(result.events.map((event) => event.id));
    return ids.has(startedId) && ids.has(finishedId);
  }, { timeout: 60_000 }).toBe(true);
  await expect(page.getByRole("heading", { name: "Change Replay" })).toBeVisible();
  const deployments = page.getByRole("list", { name: "Recorded deployments" });
  await page.getByRole("button", { name: "Refresh changes" }).click();
  await expect(deployments.getByRole("button", { name: new RegExp(service) })).toHaveCount(1, { timeout: 60_000 });
  await expect(deployments).not.toContainText("deployment.started");
  await deployments.getByRole("button", { name: new RegExp(service) }).click();
  const evidence = page.getByRole("region", { name: "Change evidence" });
  await expect(evidence.getByRole("heading", { name: `${service} deployment` })).toBeVisible();
  await expect(evidence).toContainText("Both periods are the same length");
  const comparison = evidence.getByRole("article", { name: `${service} comparison` });
  await expect(comparison).toContainText("Requests");
  await expect(comparison).toContainText("1");
  await expect(comparison).toContainText("p95 latency");
});
