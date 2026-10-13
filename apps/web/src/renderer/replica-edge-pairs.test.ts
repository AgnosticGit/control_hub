import { describe, expect, it } from "vitest";
import { edgeCardGroups } from "./replica-edge-pairs";

describe("replica edge fan-out", () => {
  it("groups every destination replica under one source so the route can branch", () => {
    const groups = edgeCardGroups(
      { node: 1, x: 10, y: 20 },
      { node: 2, x: 100, y: 20 },
      [{ node: 2, x: 100, y: 150 }, { node: 1, x: 10, y: 150 }],
    );

    expect(groups).toEqual([
      {
        source: { node: 1, x: 10, y: 20 },
        targets: [
          { point: { node: 2, x: 100, y: 20 }, isPrimary: true },
          { point: { node: 2, x: 100, y: 150 }, isPrimary: false },
        ],
        isPrimary: true,
      },
      {
        source: { node: 1, x: 10, y: 150 },
        targets: [
          { point: { node: 2, x: 100, y: 20 }, isPrimary: false },
          { point: { node: 2, x: 100, y: 150 }, isPrimary: false },
        ],
        isPrimary: false,
      },
    ]);
  });

  it("keeps one source and destination when neither endpoint has replica cards", () => {
    expect(edgeCardGroups({ node: 1, x: 0, y: 0 }, { node: 2, x: 1, y: 1 }, [])).toEqual([
      {
        source: { node: 1, x: 0, y: 0 },
        targets: [{ point: { node: 2, x: 1, y: 1 }, isPrimary: true }],
        isPrimary: true,
      },
    ]);
  });
});
