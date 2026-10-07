import { describe, expect, it } from "vitest";
import { chipFocused, frameFocused, nodeFocused, selectionAlpha, toggleSelection, type MapSelection } from "./selection-style";

describe("map selection focus", () => {
  it("keeps every placement of a selected logical service in focus", () => {
    const selection: MapSelection = { node: 4, edge: null, edgeNodes: null, host: null };
    expect(nodeFocused(4, null, selection)).toBe(true);
    expect(chipFocused({ node: 4, hostId: "vps-app" }, selection)).toBe(true);
    expect(chipFocused({ node: 4, hostId: "vps-worker" }, selection)).toBe(true);
    expect(chipFocused({ node: 2, hostId: "vps-worker" }, selection)).toBe(false);
  });

  it("keeps selected lines and their endpoint services in focus", () => {
    const selection: MapSelection = { node: null, edge: 7, edgeNodes: [2, 4], host: null };
    expect(nodeFocused(2, null, selection)).toBe(true);
    expect(nodeFocused(4, null, selection)).toBe(true);
    expect(nodeFocused(3, null, selection)).toBe(false);
    expect(selectionAlpha(selection)).toBeLessThan(1);
  });

  it("focuses only the selected machine and its replica cards", () => {
    const selection: MapSelection = { node: null, edge: null, edgeNodes: null, host: "vps-app" };
    expect(frameFocused("vps-app", selection)).toBe(true);
    expect(frameFocused("vps-worker", selection)).toBe(false);
    expect(chipFocused({ node: 4, hostId: "vps-app" }, selection)).toBe(true);
    expect(chipFocused({ node: 4, hostId: "vps-worker" }, selection)).toBe(false);
  });

  it("does not dim the map when nothing is selected", () => {
    expect(selectionAlpha({ node: null, edge: null, edgeNodes: null, host: null })).toBe(1);
  });

  it("uses the same stronger dimming for service, line, and machine selection", () => {
    expect(selectionAlpha({ node: 4, edge: null, edgeNodes: null, host: null })).toBe(0.48);
    expect(selectionAlpha({ node: null, edge: 2, edgeNodes: [1, 3], host: null })).toBe(0.48);
    expect(selectionAlpha({ node: null, edge: null, edgeNodes: null, host: "vps-app" })).toBe(0.48);
  });

  it("toggles any selected map element off and switches to another element", () => {
    expect(toggleSelection("service:orders", "service:orders")).toBeNull();
    expect(toggleSelection("line:orders:inventory", "line:orders:inventory")).toBeNull();
    expect(toggleSelection("machine:vps-app", "machine:vps-app")).toBeNull();
    expect(toggleSelection(null, "machine:vps-app")).toBe("machine:vps-app");
    expect(toggleSelection("service:orders", "line:orders:inventory")).toBe("line:orders:inventory");
  });
});
