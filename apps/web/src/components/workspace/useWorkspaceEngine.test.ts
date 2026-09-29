import { describe, expect, it } from "vitest";
import { NS_PER_MS } from "@/lib/time";
import { LIVE_ANIM_DELAY_MS, SPEEDS, animClockFor, asCompleteness, hopWindowFor, type Mode } from "./useWorkspaceEngine";

describe("animClockFor", () => {
  it("animates live requests on the wall clock, delayed for SDK export batching", () => {
    expect(animClockFor("live", 1)).toEqual({ kind: "wall", delayMs: LIVE_ANIM_DELAY_MS });
    expect(LIVE_ANIM_DELAY_MS).toBeGreaterThan(5000);
  });

  it("follows the replay speed", () => {
    for (const s of SPEEDS) expect(animClockFor("replay", s)).toEqual({ kind: "frame", rate: s });
  });

  it("freezes paused and historical views", () => {
    for (const mode of ["paused", "historical"] as const) expect(animClockFor(mode, 10)).toEqual({ kind: "frame", rate: 0 });
  });
});

describe("hopWindowFor", () => {
  it("covers the delayed live animation time", () => {
    const w = hopWindowFor("live");
    expect(w.before).toBeGreaterThanOrEqual(BigInt(LIVE_ANIM_DELAY_MS) * NS_PER_MS);
    expect(w.after).toBeGreaterThan(0n);
  });

  it("looks ahead while replaying", () => {
    const w = hopWindowFor("replay");
    expect(w.after).toBeGreaterThan(0n);
    expect(w.before).toBeGreaterThan(0n);
  });

  it("is a single instant when time stands still", () => {
    for (const mode of ["paused", "historical"] satisfies Mode[]) expect(hopWindowFor(mode)).toEqual({ before: 0n, after: 0n });
  });
});

describe("SPEEDS", () => {
  it("is strictly increasing and includes real time", () => {
    for (let i = 1; i < SPEEDS.length; i++) expect(SPEEDS[i]).toBeGreaterThan(SPEEDS[i - 1] ?? Infinity);
    expect(SPEEDS).toContain(1);
  });
});

describe("asCompleteness", () => {
  it("accepts server completeness objects", () => {
    const c = { watermark_ns: null, processing_caught_up: true, unresolved_processing_errors: 0, truncated: false, sampling: "none", notes: [] };
    expect(asCompleteness(c)).toBe(c);
  });

  it("rejects anything else", () => {
    for (const v of [null, undefined, "x", 1, [], {}, { truncated: true }]) expect(asCompleteness(v)).toBeNull();
  });
});
