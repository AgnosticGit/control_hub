import { describe, expect, it } from "vitest";
import type { InfraFrame, InfraView } from "@karbird/web-engine";
import { CHIP_H, CHIP_W, INFRA_FRAME, INFRA_HOT, INFRA_NO_METRICS, badgeText, chipAt, frameAt, frameColor, linkLabelAt } from "./infra-layer";

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
  chips: [{ node: 3, service_id: "svc_catalog", instance_id: "catalog-2", label: "catalog #2", host_id: "vps-worker", x: 700, y: 250 }],
  badges: [{ node: 3, count: 3, hosts: 2 }],
  links: [],
};

describe("infra layer hit tests", () => {
  it("finds a chip anywhere on it and nowhere outside", () => {
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

  it("labels replicas", () => {
    expect(badgeText(3, 2)).toBe("×3 · 2 machines");
    expect(badgeText(2, 1)).toBe("×2");
  });

  it("puts a link label in the middle", () => {
    expect(linkLabelAt([0, 0], [100, 50])).toEqual([50, 25]);
  });
});
