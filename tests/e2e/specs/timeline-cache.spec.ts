import { expect, test } from "./isolated-test";
import { login } from "./helpers";

const WORKSPACE = "/app/demo/local-demo";

test("timeline history persists locally and reload requests only its uncovered tail", async ({ page }) => {
  const historyLoads: Array<Array<{ from_ns: string; to_ns: string }>> = [];
  const receivedTypes = new Map<string, number>();
  page.on("websocket", (socket) => {
    socket.on("framesent", (frame) => {
      if (typeof frame.payload !== "string") return;
      try {
        const message = JSON.parse(frame.payload) as { type?: string; ranges?: Array<{ from_ns: string; to_ns: string }> };
        if (message.type === "timeline_load" && Array.isArray(message.ranges)) historyLoads.push(message.ranges);
      } catch { /* Ignore non-JSON WebSocket frames. */ }
    });
    socket.on("framereceived", (frame) => {
      if (typeof frame.payload !== "string") return;
      try {
        const message = JSON.parse(frame.payload) as { type?: string };
        if (message.type) receivedTypes.set(message.type, (receivedTypes.get(message.type) ?? 0) + 1);
      } catch { /* Ignore non-JSON WebSocket frames. */ }
    });
  });

  await login(page, "admin@karbird.local", WORKSPACE);
  await expect.poll(() => historyLoads.length).toBeGreaterThan(0);
  await expect.poll(() => receivedTypes.get("timeline_complete") ?? 0).toBeGreaterThan(0);
  const readCacheCounts = () => page.evaluate(async () => {
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const request = indexedDB.open("karbird-timeline");
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    const tx = db.transaction(["chunks", "ranges"], "readonly");
    const counts = await Promise.all([
      new Promise<number>((resolve, reject) => { const r = tx.objectStore("chunks").count(); r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); }),
      new Promise<number>((resolve, reject) => { const r = tx.objectStore("ranges").count(); r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); }),
    ]);
    db.close();
    return { counts, version: db.version };
  });
  await expect.poll(async () => (await readCacheCounts()).counts.every((count) => count > 0)).toBe(true);

  const firstRequestCount = historyLoads.length;
  await page.reload();
  await expect.poll(() => historyLoads.length).toBeGreaterThan(firstRequestCount);
  await expect.poll(() => receivedTypes.get("timeline_complete") ?? 0).toBeGreaterThan(1);
  const reloadedRanges = historyLoads.slice(firstRequestCount).flat();
  const requestedNs = reloadedRanges.reduce((sum, range) => sum + BigInt(range.to_ns) - BigInt(range.from_ns), 0n);
  expect(requestedNs).toBeLessThan(60n * 1_000_000_000n);

  await page.goto("/");
  await page.evaluate(async () => {
    await new Promise<void>((resolve, reject) => {
      const deletion = indexedDB.deleteDatabase("karbird-timeline");
      deletion.onsuccess = () => resolve();
      deletion.onerror = () => reject(deletion.error);
      deletion.onblocked = () => reject(new Error("timeline cache still has an open connection"));
    });
    await new Promise<void>((resolve, reject) => {
      const old = indexedDB.open("karbird-timeline", 1);
      old.onupgradeneeded = () => {
        const db = old.result;
        db.createObjectStore("chunks", { keyPath: "id" }).put({ id: "old-schema-marker" });
        db.createObjectStore("ranges", { keyPath: "id" }).put({ id: "old-schema-marker" });
        db.createObjectStore("meta", { keyPath: "id" }).put({ id: "size", bytes: 123 });
      };
      old.onsuccess = () => { old.result.close(); resolve(); };
      old.onerror = () => reject(old.error);
    });
  });
  await page.routeWebSocket("**/api/v1/projects/demo/environments/local-demo/live**", (socket) => socket.close());
  await page.goto(WORKSPACE);
  await expect(page.getByLabel("Timeline")).toBeVisible();
  await expect.poll(async () => {
    const current = await readCacheCounts();
    const marker = await page.evaluate(async () => {
      const db = await new Promise<IDBDatabase>((resolve, reject) => {
        const request = indexedDB.open("karbird-timeline");
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      const tx = db.transaction(["chunks", "ranges"], "readonly");
      const found = await Promise.all(["chunks", "ranges"].map((store) => new Promise<boolean>((resolve, reject) => {
        const request = tx.objectStore(store).get("old-schema-marker");
        request.onsuccess = () => resolve(request.result !== undefined);
        request.onerror = () => reject(request.error);
      })));
      db.close();
      return found.some(Boolean);
    });
    return current.version > 1 && !marker;
  }).toBe(true);
});
