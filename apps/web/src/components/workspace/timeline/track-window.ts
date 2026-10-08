import type { View } from "./scale";

export interface StableTrackWindow extends View {
  buckets: number;
  bucketNs: bigint;
}

function ceilDiv(value: bigint, divisor: bigint): bigint {
  return value <= 0n ? 0n : (value + divisor - 1n) / divisor;
}

function floorDiv(value: bigint, divisor: bigint): bigint {
  const quotient = value / divisor;
  return value < 0n && value % divisor !== 0n ? quotient - 1n : quotient;
}

/**
 * Align a requested range to an epoch-anchored bucket grid. Live windows can slide every second;
 * without this alignment the engine repartitions the same timestamps into different buckets.
 */
export function stableTrackWindow(view: View, targetBuckets: number): StableTrackWindow {
  const span = view.to > view.from ? view.to - view.from : 1n;
  const count = BigInt(Math.max(1, Math.floor(targetBuckets)));
  const bucketNs = ceilDiv(span, count) || 1n;
  const from = floorDiv(view.from, bucketNs) * bucketNs;
  const buckets = Number(ceilDiv(view.to - from, bucketNs));
  return { from, to: from + BigInt(buckets) * bucketNs, buckets, bucketNs };
}
