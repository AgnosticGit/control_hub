import { describe, expect, it } from "vitest";
import { edgeBranchPaths } from "./replica-edge-paths";
import type { EdgeCardGroup } from "./replica-edge-pairs";

const group: EdgeCardGroup = {
  source: { node: 1, x: 0, y: 0 },
  targets: [
    { point: { node: 2, x: 120, y: 0 }, isPrimary: true },
    { point: { node: 2, x: 120, y: 120 }, isPrimary: false },
  ],
  isPrimary: true,
};

describe("replica edge branches", () => {
  it.each(["curved", "orthogonal", "metro"] as const)("%s routes the shared trunk and branches around unrelated service cards", (style) => {
    const obstructed: EdgeCardGroup = {
      source: { node: 1, x: 0, y: 0 },
      targets: [
        { point: { node: 2, x: 160, y: -40 }, isPrimary: true },
        { point: { node: 2, x: 160, y: 40 }, isPrimary: false },
      ],
      isPrimary: true,
    };
    const card = { node: 3, x: 40, y: 0, box: { hw: 20, hh: 20 } };
    const paths = edgeBranchPaths(obstructed, style, () => ({ hw: 20, hh: 12 }), [card]);
    const allPaths = [paths?.trunk, ...(paths?.branches.map(({ path }) => path) ?? [])];

    expect(allPaths.every((path) => path && pathAvoidsBox(path.pts, card)), JSON.stringify(allPaths.map((path) => path?.pts))).toBe(true);
  });

  it("uses one trunk from the source and branches to every destination replica", () => {
    const paths = edgeBranchPaths(group, "orthogonal", () => ({ hw: 20, hh: 12 }));

    expect(paths).not.toBeNull();
    expect(paths?.trunk.pts.slice(0, 2)).not.toEqual([0, 0]);
    expect(paths?.branches).toHaveLength(2);
    const junction = paths?.trunk.pts.slice(-2);
    expect(paths?.branches.map(({ path }) => path.pts.slice(0, 2))).toEqual([junction, junction]);
    const ends = paths?.branches.map(({ path }) => path.pts.slice(-2));
    expect(ends?.[0]).not.toEqual([120, 0]);
    expect(ends?.[1]).not.toEqual([120, 120]);
    expect(ends?.[0]).not.toEqual(ends?.[1]);
  });

  it("does not create a branch for a single destination", () => {
    const firstTarget = group.targets[0];
    if (!firstTarget) throw new Error("test group needs one target");
    const single: EdgeCardGroup = { ...group, targets: [firstTarget] };
    expect(edgeBranchPaths(single, "curved", () => ({ hw: 20, hh: 12 }))).toBeNull();
  });
});

function pathAvoidsBox(points: number[], box: { x: number; y: number; box: { hw: number; hh: number } }): boolean {
  for (let i = 0; i + 3 < points.length; i += 2) {
    const x0 = points[i] ?? 0;
    const y0 = points[i + 1] ?? 0;
    const x1 = points[i + 2] ?? 0;
    const y1 = points[i + 3] ?? 0;
    const minX = box.x - box.box.hw;
    const maxX = box.x + box.box.hw;
    const minY = box.y - box.box.hh;
    const maxY = box.y + box.box.hh;
    let t0 = 0;
    let t1 = 1;
    let possible = true;
    const dx = x1 - x0;
    const dy = y1 - y0;
    const clips: Array<[number, number]> = [[-dx, x0 - minX], [dx, maxX - x0], [-dy, y0 - minY], [dy, maxY - y0]];
    for (const [p, q] of clips) {
      if (p === 0) { if (q <= 0) { possible = false; break; } continue; }
      const t = q / p;
      if (p < 0) t0 = Math.max(t0, t);
      else t1 = Math.min(t1, t);
      if (t0 >= t1) { possible = false; break; }
    }
    if (possible && t0 < t1 && t1 > 0 && t0 < 1) return false;
  }
  return true;
}
