// Tests for `@controlhub/web-engine` constants (the package has no test runner of its own).
// The render-buffer layout is shared with Rust; a drift here corrupts every frame silently.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import * as engine from "@controlhub/web-engine";
import { describe, expect, it } from "vitest";

const repo = (rel: string) => readFileSync(fileURLToPath(new URL(`../../../../${rel}`, import.meta.url)), "utf8");

function rustConst(source: string, name: string): number {
  const m = new RegExp(`pub const ${name}: \\w+ = (\\d+);`).exec(source);
  if (!m?.[1]) throw new Error(`${name} not found in Rust source`);
  return Number(m[1]);
}

const isPowerOfTwo = (n: number) => Number.isInteger(n) && n > 0 && (n & (n - 1)) === 0;

const FLAGS = {
  FLAG_HIGHLIGHT: engine.FLAG_HIGHLIGHT,
  FLAG_INFERRED: engine.FLAG_INFERRED,
  FLAG_DECLARED: engine.FLAG_DECLARED,
  FLAG_RESOURCE: engine.FLAG_RESOURCE,
  FLAG_PROBLEM: engine.FLAG_PROBLEM,
  FLAG_DIMMED: engine.FLAG_DIMMED,
};
const HOP_FLAGS = {
  HOP_ERROR: engine.HOP_ERROR,
  HOP_LEAF: engine.HOP_LEAF,
  HOP_HIGHLIGHT: engine.HOP_HIGHLIGHT,
  HOP_CLAMPED: engine.HOP_CLAMPED,
  HOP_DIMMED: engine.HOP_DIMMED,
};

describe("render buffer flags", () => {
  for (const [group, flags] of Object.entries({ node_edge: FLAGS, hop: HOP_FLAGS })) {
    it(`${group} flags are distinct powers of two`, () => {
      const values = Object.values(flags);
      for (const v of values) expect(isPowerOfTwo(v)).toBe(true);
      expect(new Set(values).size).toBe(values.length);
      // Fits exactly into a float32 without rounding.
      expect(values.reduce((a, b) => a | b, 0)).toBeLessThan(2 ** 24);
    });
  }
});

describe("strides and status", () => {
  it("has the documented strides", () => {
    expect(engine.NODE_STRIDE).toBe(7);
    expect(engine.EDGE_STRIDE).toBe(8);
    expect(engine.HOP_STRIDE).toBe(7);
  });

  it("encodes health status as 0..3", () => {
    expect(Object.values(engine.STATUS).sort()).toEqual([0, 1, 2, 3]);
    expect(engine.STATUS.unknown).toBe(0);
  });

  it("ABI version is a positive integer", () => {
    expect(Number.isInteger(engine.ENGINE_ABI_VERSION)).toBe(true);
    expect(engine.ENGINE_ABI_VERSION).toBeGreaterThan(0);
  });
});

describe("matches the Rust engine", () => {
  const core = repo("crates/controlhub-engine-core/src/lib.rs");
  const hops = repo("crates/controlhub-engine-core/src/hops.rs");
  const wasm = repo("crates/controlhub-web-engine/src/lib.rs");

  it("node/edge strides and flags", () => {
    expect(rustConst(core, "NODE_STRIDE")).toBe(engine.NODE_STRIDE);
    expect(rustConst(core, "EDGE_STRIDE")).toBe(engine.EDGE_STRIDE);
    for (const [name, v] of Object.entries(FLAGS)) expect(rustConst(core, name), name).toBe(v);
  });

  it("hop stride and flags", () => {
    expect(rustConst(hops, "HOP_STRIDE")).toBe(engine.HOP_STRIDE);
    for (const [name, v] of Object.entries(HOP_FLAGS)) expect(rustConst(hops, name), name).toBe(v);
  });

  it("ABI version", () => {
    const m = /pub fn engine_abi_version\(\) -> u32 \{\s*(\d+)\s*\}/.exec(wasm);
    expect(Number(m?.[1])).toBe(engine.ENGINE_ABI_VERSION);
  });
});
