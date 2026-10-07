import { describe, expect, it } from "vitest";
import type { InfraFrame, InfraView } from "@karbird/web-engine";
import {
  CHIP_H,
  CHIP_W,
  INFRA_FRAME,
  INFRA_HOT,
  INFRA_NO_METRICS,
  INFRA_SELECTED,
  badgeText,
  chipAt,
  frameAt,
  frameColor,
  frameLook,
  linkArc,
  linkLabelAt,
} from "./infra-layer";

const frame = (over: Partial<InfraFrame> = {}): InfraFrame => ({
  host_id: "vps-app",
  label: "vps-app · CPU 92% · RAM 870/1024 MB",
  x: 0,
  y: 0,
  width: 500,
  height: 300,
  hot: false,
  has_metrics: true,
  ...over,
});

const view: InfraView = {
  frames: [frame(), frame({ host_id: "vps-worker", x: 600 })],
  chips: [{ node: 3, service_id: "svc_catalog", instance_id: "catalog-2", instance_ids: ["catalog-2", "catalog-3"], label: "catalog", host_id: "vps-worker", x: 700, y: 250 }],
  badges: [{ node: 3, count: 3, hosts: 2 }],
  links: [],
};

describe("infra layer hit tests", () => {
  it("finds a replica card anywhere on it and nowhere outside", () => {
    expect(chipAt(view, 700, 250)?.instance_id).toBe("catalog-2");
    expect(chipAt(view, 700 + CHIP_W / 2, 250 + CHIP_H / 2)?.node).toBe(3);
    expect(chipAt(view, 700 + CHIP_W / 2 + 1, 250)).toBeNull();
    expect(chipAt(null, 700, 250)).toBeNull();
  });

  it("finds the frame containing a point", () => {
    expect(frameAt(view, 10, 10)?.host_id).toBe("vps-app");
    expect(frameAt(view, 650, 299)?.host_id).toBe("vps-worker");
    expect(frameAt(view, 550, 10)).toBeNull();
    expect(frameAt(undefined, 10, 10)).toBeNull();
  });
});

describe("infra layer styling", () => {
  it("colours hot and agent-less machines differently", () => {
    expect(frameColor(frame())).toBe(INFRA_FRAME);
    expect(frameColor(frame({ hot: true }))).toBe(INFRA_HOT);
    expect(frameColor(frame({ hot: true, has_metrics: false }))).toBe(INFRA_NO_METRICS);
  });

  it("highlights the selected machine over hot, agent-less and plain frames", () => {
    for (const f of [frame(), frame({ hot: true }), frame({ has_metrics: false })]) {
      const sel = frameLook(f, true);
      const plain = frameLook(f, false);
      expect(sel.color).toBe(INFRA_SELECTED);
      expect(plain.color).toBe(frameColor(f));
      expect(sel.strokeWidth).toBeGreaterThan(plain.strokeWidth);
      expect(sel.fillAlpha).toBeGreaterThan(plain.fillAlpha);
      expect(sel.halo).toBeGreaterThan(0);
      expect(plain.halo).toBe(0);
    }
  });

  it("labels replicas", () => {
    expect(badgeText(3, 2)).toBe("×3 · 2 machines");
    expect(badgeText(2, 1)).toBe("×2");
  });

  it("draws a link as an arc above the straight line and labels its top", () => {
    const from: [number, number] = [0, 100];
    const to: [number, number] = [300, 100];
    const arc = linkArc(from, to);
    expect(arc.slice(0, 2)).toEqual(from);
    expect(arc.slice(-2)).toEqual(to);
    // Every inner point is well above the straight call edge between the same cards.
    for (let i = 2; i + 2 < arc.length - 2; i += 2) expect(arc[i + 1] ?? 0).toBeLessThan(100 - 10);
    const [mx, my] = linkLabelAt(from, to);
    expect(mx).toBeCloseTo(150);
    const top = Math.min(...arc.filter((_, i) => i % 2 === 1));
    expect(my).toBeCloseTo(top, 0);
    // Same bow side whichever way the link points; a zero-length link still has an arc.
    expect(linkLabelAt(to, from)[1]).toBeCloseTo(my);
    expect(linkArc(from, from).length).toBeGreaterThan(4);
  });
});
