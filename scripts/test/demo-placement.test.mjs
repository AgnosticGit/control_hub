import { test } from "node:test";
import assert from "node:assert/strict";
import { CATALOG_PORTS, demoContainerId, demoPlacement } from "../demo-placement.mjs";

test("every demo service sits in exactly one container on one machine", () => {
  const names = demoPlacement().flatMap((h) => h.containers.filter((c) => c.url).map((c) => c.name));
  assert.deepEqual(
    [...names].sort(),
    ["auth", "catalog-1", "catalog-2", "catalog-3", "gateway", "inventory", "notifications", "orders", "payments"],
  );
});

test("catalog replicas are spread over both machines, orders and inventory share vps-app", () => {
  const hostOf = new Map(demoPlacement().flatMap((h) => h.containers.map((c) => [c.name, h.host])));
  assert.deepEqual(new Set(["catalog-1", "catalog-2", "catalog-3"].map((n) => hostOf.get(n))), new Set(["vps-app", "vps-worker"]));
  assert.equal(hostOf.get("orders"), "vps-app");
  assert.equal(hostOf.get("inventory"), "vps-app");
  const app = demoPlacement().find((h) => h.host === "vps-app");
  assert.deepEqual([app?.cores, app?.memoryMb], [1, 1024]);
});

test("container IDs are stable 64-hex values and distinct per instance", () => {
  assert.match(demoContainerId("orders"), /^[0-9a-f]{64}$/);
  assert.equal(demoContainerId("orders"), demoContainerId("orders"));
  const ids = demoPlacement().flatMap((h) => h.containers.map((c) => c.id));
  assert.equal(new Set(ids).size, ids.length);
});

test("the demo Postgres is a synthetic postgres container on vps-app, next to its callers", () => {
  const placement = demoPlacement();
  const app = placement.find((h) => h.host === "vps-app");
  const pg = app?.containers.find((c) => c.name === "postgres");
  assert.ok(pg, "postgres container on vps-app");
  assert.equal(pg.url, undefined, "not measured from a demo service");
  assert.match(pg.image, /^postgres:\d+/);
  assert.equal(pg.service, "postgres");
  assert.ok(pg.synthetic.cores > 0 && pg.synthetic.rssMb > 0);
  const others = placement.flatMap((h) => h.containers).filter((c) => c !== pg);
  assert.ok(others.every((c) => !c.image?.startsWith("postgres")), "one postgres image, so the loopback rule is unambiguous");
});

test("catalog replica URLs use the catalog ports", () => {
  const urls = demoPlacement()
    .flatMap((h) => h.containers)
    .filter((c) => c.name.startsWith("catalog-"))
    .map((c) => Number(new URL(c.url).port))
    .sort();
  assert.deepEqual(urls, [...CATALOG_PORTS].sort());
});
