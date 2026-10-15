import type { InfrastructureSnapshot } from "@karbird/web-engine";

export const INFRA_SNAPSHOT_CACHE_LIMIT = 128;

/** Keeps historical machine snapshots available while scrubbing back and forth. */
export class InfrastructureSnapshotCache {
  private readonly snapshots = new Map<string, InfrastructureSnapshot>();

  get(key: string): InfrastructureSnapshot | null {
    const snapshot = this.snapshots.get(key);
    if (!snapshot) return null;
    this.snapshots.delete(key);
    this.snapshots.set(key, snapshot);
    return snapshot;
  }

  set(key: string, snapshot: InfrastructureSnapshot): void {
    this.snapshots.delete(key);
    this.snapshots.set(key, snapshot);
    while (this.snapshots.size > INFRA_SNAPSHOT_CACHE_LIMIT) {
      const oldest = this.snapshots.keys().next().value;
      if (oldest === undefined) break;
      this.snapshots.delete(oldest);
    }
  }
}
