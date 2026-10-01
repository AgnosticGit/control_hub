import { describe, expect, it } from "vitest";
import { WorkspaceMemory } from "./workspace-memory";

describe("WorkspaceMemory", () => {
  it("starts empty, so a first visit fits the map and the timeline", () => {
    const m = new WorkspaceMemory();
    expect(m.camera()).toBeNull();
    expect(m.timelineView()).toBeNull();
  });

  it("returns the saved camera and timeline zoom", () => {
    const m = new WorkspaceMemory();
    m.saveCamera({ x: -120, y: 40, scale: 1.5 });
    m.saveTimelineView({ from: 10n, to: 20n });
    expect(m.camera()).toEqual({ x: -120, y: 40, scale: 1.5 });
    expect(m.timelineView()).toEqual({ from: 10n, to: 20n });
    m.saveTimelineView(null);
    expect(m.timelineView()).toBeNull();
  });

  it("drops a broken camera instead of restoring an unusable map", () => {
    const m = new WorkspaceMemory();
    for (const bad of [
      { x: Number.NaN, y: 0, scale: 1 },
      { x: 0, y: Infinity, scale: 1 },
      { x: 0, y: 0, scale: 0 },
      { x: 0, y: 0, scale: -2 },
    ]) {
      m.saveCamera({ x: 1, y: 1, scale: 1 });
      m.saveCamera(bad);
      expect(m.camera()).toBeNull();
    }
  });
});
