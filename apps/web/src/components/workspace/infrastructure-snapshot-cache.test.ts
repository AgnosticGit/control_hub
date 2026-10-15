import { describe, expect, it } from "vitest";
import type { InfrastructureSnapshot } from "@karbird/web-engine";
import { InfrastructureSnapshotCache, INFRA_SNAPSHOT_CACHE_LIMIT } from "./infrastructure-snapshot-cache";

const snapshot = (name: string) => ({ hosts: [{ name }] }) as InfrastructureSnapshot;

describe("InfrastructureSnapshotCache", () => {
  it("reuses prior snapshots and refreshes their LRU position", () => {
    const cache = new InfrastructureSnapshotCache();
    cache.set("a", snapshot("a"));
    cache.set("b", snapshot("b"));
    expect(cache.get("a")?.hosts[0]?.name).toBe("a");
    for (let i = 0; i < INFRA_SNAPSHOT_CACHE_LIMIT - 1; i += 1) cache.set(`n${i}`, snapshot(`n${i}`));
    expect(cache.get("a")?.hosts[0]?.name).toBe("a");
    expect(cache.get("b")).toBeNull();
  });
});
