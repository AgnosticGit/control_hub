import { afterEach, describe, expect, it, vi } from "vitest";
import {
  NS_PER_MS,
  NS_PER_SEC,
  clampNs,
  formatClock,
  formatClockMicros,
  formatDateTime,
  formatDurationNs,
  nowNs,
  parseNs,
} from "./time";

const pad = (n: number, w = 2) => String(n).padStart(w, "0");

describe("nowNs", () => {
  afterEach(() => vi.useRealTimers());

  it("is Date.now() in nanoseconds", () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_790_000_000_123);
    expect(nowNs()).toBe(1_790_000_000_123n * NS_PER_MS);
  });
});

describe("parseNs", () => {
  it("parses decimal integers without losing precision past 2^53", () => {
    expect(parseNs("0")).toBe(0n);
    expect(parseNs("-42")).toBe(-42n);
    expect(parseNs("1790000000123456789")).toBe(1_790_000_000_123_456_789n);
    expect(parseNs("9223372036854775807")).toBe(9_223_372_036_854_775_807n);
  });

  it("rejects everything that is not a plain decimal integer", () => {
    for (const bad of [null, undefined, "", " 1", "1 ", "1.5", "1e9", "0x10", "abc", "+1", "--1", "1_000"]) {
      expect(parseNs(bad)).toBeNull();
    }
  });
});

describe("formatClock", () => {
  it("formats local wall-clock time with and without milliseconds", () => {
    const ms = Date.UTC(2026, 8, 29, 4, 5, 6, 7);
    const d = new Date(ms);
    const hms = `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
    const ns = BigInt(ms) * NS_PER_MS + 999_999n;
    expect(formatClock(ns)).toBe(`${hms}.007`);
    expect(formatClock(ns, false)).toBe(hms);
  });

  it("shows microseconds separately", () => {
    const ms = Date.UTC(2026, 0, 1, 0, 0, 0, 250);
    const ns = BigInt(ms) * NS_PER_MS + 42_999n;
    expect(formatClockMicros(ns)).toBe(`${formatClock(ns)} 042`);
    expect(formatClockMicros(BigInt(ms) * NS_PER_MS)).toMatch(/ 000$/);
  });

  it("keeps microseconds non-negative before the epoch", () => {
    expect(formatClockMicros(-1n)).toMatch(/ 999$/);
  });

  it("formatDateTime prefixes the local date", () => {
    const ms = Date.UTC(2026, 8, 29, 12, 0, 0, 0);
    const ns = BigInt(ms) * NS_PER_MS;
    expect(formatDateTime(ns)).toBe(`${new Date(ms).toLocaleDateString()} ${formatClock(ns)}`);
  });
});

describe("formatDurationNs", () => {
  it("picks µs, ms or s", () => {
    expect(formatDurationNs(0n)).toBe("0 µs");
    expect(formatDurationNs(250_000n)).toBe("250 µs");
    expect(formatDurationNs(NS_PER_MS)).toBe("1.00 ms");
    expect(formatDurationNs(9_990_000n)).toBe("9.99 ms");
    expect(formatDurationNs(123_400_000n)).toBe("123.4 ms");
    expect(formatDurationNs(NS_PER_SEC)).toBe("1.00 s");
    expect(formatDurationNs(90n * NS_PER_SEC)).toBe("90.00 s");
  });
});

describe("clampNs", () => {
  it("clamps into [lo, hi]", () => {
    expect(clampNs(5n, 0n, 10n)).toBe(5n);
    expect(clampNs(-1n, 0n, 10n)).toBe(0n);
    expect(clampNs(11n, 0n, 10n)).toBe(10n);
    expect(clampNs(0n, 0n, 10n)).toBe(0n);
    expect(clampNs(10n, 0n, 10n)).toBe(10n);
  });
});
