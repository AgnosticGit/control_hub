// Nanosecond time helpers. Timestamps are BigInt in the UI and decimal
// strings on the wire; JS numbers are only used for display math on small
// differences.

export const NS_PER_MS = 1_000_000n;
export const NS_PER_SEC = 1_000_000_000n;

export function nowNs(): bigint {
  return BigInt(Date.now()) * NS_PER_MS;
}

export function parseNs(s: string | null | undefined): bigint | null {
  if (!s || !/^-?\d+$/.test(s)) return null;
  return BigInt(s);
}

export function formatClock(ns: bigint, withMs = true): string {
  const ms = Number(ns / NS_PER_MS);
  const d = new Date(ms);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  const ss = String(d.getSeconds()).padStart(2, "0");
  if (!withMs) return `${hh}:${mm}:${ss}`;
  return `${hh}:${mm}:${ss}.${String(d.getMilliseconds()).padStart(3, "0")}`;
}

/** `HH:MM:SS.mmm uuu` — microseconds shown when replaying slowly. */
export function formatClockMicros(ns: bigint): string {
  const us = ((ns % NS_PER_MS) + NS_PER_MS) % NS_PER_MS / 1000n;
  return `${formatClock(ns)} ${String(us).padStart(3, "0")}`;
}

export function formatDateTime(ns: bigint): string {
  const d = new Date(Number(ns / NS_PER_MS));
  return `${d.toLocaleDateString()} ${formatClock(ns)}`;
}

/** Format an optional API timestamp using the browser's locale. */
export function formatIsoDateTime(iso: string | null | undefined): string {
  return iso ? new Date(iso).toLocaleString() : "—";
}

export function formatIsoDate(iso: string | null | undefined): string {
  return iso ? new Date(iso).toLocaleDateString() : "—";
}

export function formatDurationNs(ns: bigint): string {
  const ms = Number(ns) / 1e6;
  if (ms < 1) return `${(ms * 1000).toFixed(0)} µs`;
  if (ms < 1000) return `${ms.toFixed(ms < 10 ? 2 : 1)} ms`;
  return `${(ms / 1000).toFixed(2)} s`;
}

export function clampNs(v: bigint, lo: bigint, hi: bigint): bigint {
  return v < lo ? lo : v > hi ? hi : v;
}
