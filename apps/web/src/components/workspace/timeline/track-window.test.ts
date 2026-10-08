import { describe, expect, it } from "vitest";
import { stableTrackWindow } from "./track-window";

const BUCKET = 5_000_000_000n;
const SPAN = 900n * 1_000_000_000n;

describe("stableTrackWindow", () => {
  it("keeps a fixed epoch-aligned bucket grid as a live range advances", () => {
    const first = stableTrackWindow({ from: 1_000_000_001n, to: 1_000_000_001n + SPAN }, 180);
    const next = stableTrackWindow({ from: 1_001_000_001n, to: 1_001_000_001n + SPAN }, 180);

    expect(first.bucketNs).toBe(BUCKET);
    expect(next.bucketNs).toBe(BUCKET);
    expect(first.from).toBe(next.from);
    expect(first.to).toBe(next.to);
    expect(first.buckets).toBe(next.buckets);
  });

  it("slides only by whole buckets and covers the visible range", () => {
    const first = stableTrackWindow({ from: 1n, to: 1n + SPAN }, 180);
    const next = stableTrackWindow({ from: BUCKET + 1n, to: BUCKET + 1n + SPAN }, 180);

    expect(next.from - first.from).toBe(BUCKET);
    expect(next.to - first.to).toBe(BUCKET);
    expect(next.from).toBeLessThanOrEqual(BUCKET + 1n);
    expect(next.to).toBeGreaterThanOrEqual(BUCKET + 1n + SPAN);
  });
});
