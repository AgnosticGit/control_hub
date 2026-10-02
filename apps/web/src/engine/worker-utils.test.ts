import { describe, expect, it } from "vitest";
import { countItems, liveRetryDelayMs, newerSeek, parseLiveHead, timelineTail } from "./worker-utils";

describe("timelineTail", () => {
  it("reads the metadata after the items array without parsing the items", () => {
    // The items part is deliberately invalid JSON: only the tail may be parsed.
    const text =
      '{"items":[{"type":"span", BROKEN ],"from_ns":"100","to_ns":"200","spans_truncated":true,"logs_truncated":false,"completeness":{"processing_caught_up":true}}';
    expect(timelineTail(text)).toEqual({
      from_ns: "100",
      to_ns: "200",
      spans_truncated: true,
      logs_truncated: false,
      completeness: { processing_caught_up: true },
    });
  });

  it("reads where the log items came from", () => {
    const text = '{"items":[ BROKEN ],"from_ns":"1","to_ns":"2","logs_truncated":true,"logs_source":"mixed","completeness":{}}';
    expect(timelineTail(text)).toMatchObject({ logs_source: "mixed", logs_truncated: true });
  });

  it("uses the last marker when an item contains the same characters", () => {
    const text = '{"items":[{"type":"log","body":"],\\"from_ns\\":"}],"from_ns":"1","to_ns":"2"}';
    expect(timelineTail(text)).toMatchObject({ from_ns: "1", to_ns: "2" });
  });

  it("falls back to a full parse for other field orders", () => {
    expect(timelineTail('{"from_ns":"5","items":[],"to_ns":"6"}')).toMatchObject({ from_ns: "5", to_ns: "6" });
  });

  it("throws on invalid JSON without a tail", () => {
    expect(() => timelineTail("not json")).toThrow();
  });
});

describe("countItems", () => {
  it("counts typed items", () => {
    expect(countItems('{"items":[]}')).toBe(0);
    expect(countItems('{"items":[{"type":"span"},{"type":"log"},{"type":"event"}]}')).toBe(3);
  });

  it("counts a live batch including its own type field", () => {
    expect(countItems('{"type":"batch","resume":"9","items":[{"type":"span"}]}')).toBe(2);
  });
});

describe("parseLiveHead", () => {
  it("reads type and resume token", () => {
    expect(parseLiveHead('{"type":"batch","resume":"42","items":[]}')).toEqual({ type: "batch", resume: "42" });
    expect(parseLiveHead('{"type":"heartbeat"}')).toEqual({ type: "heartbeat", resume: undefined });
  });

  it("ignores messages that do not start with the expected head", () => {
    expect(parseLiveHead('{"resume":"1","type":"batch"}')).toBeNull();
    expect(parseLiveHead(' {"type":"batch"}')).toBeNull();
    expect(parseLiveHead('{"type":"<script>"}')).toBeNull();
    expect(parseLiveHead("")).toBeNull();
  });

  it("only accepts numeric resume tokens", () => {
    expect(parseLiveHead('{"type":"gap","resume":"abc"}')).toEqual({ type: "gap", resume: undefined });
  });

  it("only looks at the first 128 characters", () => {
    expect(parseLiveHead(`${" ".repeat(128)}{"type":"batch"}`)).toBeNull();
  });
});

describe("liveRetryDelayMs", () => {
  it("backs off exponentially with jitter between 50 % and 100 %", () => {
    expect(liveRetryDelayMs(1, 1)).toBe(1000);
    expect(liveRetryDelayMs(1, 0)).toBe(500);
    expect(liveRetryDelayMs(2, 1)).toBe(2000);
    expect(liveRetryDelayMs(5, 1)).toBe(16_000);
  });

  it("is capped at 30 s", () => {
    expect(liveRetryDelayMs(6, 1)).toBe(30_000);
    expect(liveRetryDelayMs(100, 1)).toBe(30_000);
    expect(liveRetryDelayMs(100, 0)).toBe(15_000);
  });
});

describe("newerSeek", () => {
  const s = (generation: number) => ({ timeNs: String(generation), generation });

  it("keeps the newest pending seek", () => {
    expect(newerSeek(null, s(1))).toEqual(s(1));
    expect(newerSeek(s(1), s(2))).toEqual(s(2));
    expect(newerSeek(s(3), s(2))).toEqual(s(3));
    expect(newerSeek(s(3), { timeNs: "x", generation: 3 })).toEqual({ timeNs: "x", generation: 3 });
  });
});
