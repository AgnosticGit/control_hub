import type { Schemas } from "@/lib/api";
import { NS_PER_SEC } from "@/lib/time";

type Service = Schemas["ServiceDto"];
type Completeness = Schemas["Completeness"];

export interface ComparisonWindow {
  before: { from: bigint; to: bigint };
  after: { from: bigint; to: bigint };
  provisional: boolean;
}

const DEFAULT_DURATION = 15n * 60n * NS_PER_SEC;

/** Keep both samples the same length, including while a recent release is still unfolding. */
export function comparisonWindow(at: bigint, now: bigint, maxDuration = DEFAULT_DURATION): ComparisonWindow | null {
  if (at >= now || maxDuration <= 0n) return null;
  const duration = now - at < maxDuration ? now - at : maxDuration;
  if (at < duration) return null;
  return {
    before: { from: at - duration, to: at },
    after: { from: at, to: at + duration },
    provisional: duration < maxDuration,
  };
}

export interface ServiceComparison {
  serviceId: string;
  name: string;
  before: Service | null;
  after: Service | null;
  errorRateBefore: number | null;
  errorRateAfter: number | null;
  p95DeltaMs: number | null;
}

const errorRate = (service: Service | null): number | null => service && service.requests > 0
  ? service.errors / service.requests * 100
  : null;

/** Service identity is stable across release versions; missing samples remain explicit. */
export function compareServices(primaryServiceId: string, before: Service[], after: Service[]): ServiceComparison[] {
  const prior = new Map(before.map((service) => [service.service_id, service]));
  const later = new Map(after.map((service) => [service.service_id, service]));
  const ids = new Set([...prior.keys(), ...later.keys()]);
  return [...ids].map((serviceId) => {
    const first = prior.get(serviceId) ?? null;
    const second = later.get(serviceId) ?? null;
    return {
      serviceId,
      name: second?.name ?? first?.name ?? serviceId,
      before: first,
      after: second,
      errorRateBefore: errorRate(first),
      errorRateAfter: errorRate(second),
      p95DeltaMs: first?.p95_ms != null && second?.p95_ms != null ? second.p95_ms - first.p95_ms : null,
    };
  }).sort((a, b) => Number(b.serviceId === primaryServiceId) - Number(a.serviceId === primaryServiceId)
    || Number(b.before !== null) - Number(a.before !== null)
    || a.name.localeCompare(b.name));
}

/** Conditions that make an apparently precise before/after difference less trustworthy. */
export function comparisonWarnings(before: Completeness, after: Completeness, window: ComparisonWindow): string[] {
  const warnings: string[] = [];
  if (window.provisional) warnings.push("The post-release window is still open; this comparison will change.");
  const watermarks = [before.watermark_ns, after.watermark_ns];
  if (!before.processing_caught_up || !after.processing_caught_up || watermarks.some((value) => value == null || BigInt(value) < window.after.to)) {
    warnings.push("Processing has not reached the end of the comparison window.");
  }
  if (before.unresolved_processing_errors > 0 || after.unresolved_processing_errors > 0) {
    warnings.push("Some records failed processing and are absent from this comparison.");
  }
  if (before.truncated || after.truncated) warnings.push("One comparison window was truncated.");
  return warnings;
}
