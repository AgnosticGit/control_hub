import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readPersisted } from "@/lib/persisted";
import { EDGE_STYLES } from "@/renderer/edge-path";
import { EDGE_STYLE_KEY, parseEdgeStyle, setEdgeStyle } from "./edgeStyleStore";

beforeEach(() => {
  const data = new Map<string, string>();
  const win = Object.assign(new EventTarget(), {
    localStorage: {
      getItem: (k: string) => data.get(k) ?? null,
      setItem: (k: string, v: string) => void data.set(k, v),
    },
  });
  vi.stubGlobal("window", win);
});

afterEach(() => vi.unstubAllGlobals());

describe("edge style preference", () => {
  it("accepts every known style", () => {
    for (const s of EDGE_STYLES) expect(parseEdgeStyle(s)).toBe(s);
  });

  it("rejects stale or hand-edited values so the default is used", () => {
    for (const raw of ["", "Curved", "straight", " metro", "__proto__", "null"]) {
      expect(parseEdgeStyle(raw)).toBeNull();
    }
  });

  it("stores the chosen style under a namespaced key", () => {
    expect(EDGE_STYLE_KEY).toBe("karbird.map.edgeStyle");
    setEdgeStyle("orthogonal");
    expect(readPersisted(EDGE_STYLE_KEY)).toBe("orthogonal");
  });
});
