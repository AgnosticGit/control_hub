import type { TimelineTracks } from "@karbird/web-engine";
import type { Schemas } from "@/lib/api";

const MAX_TRACK_COUNT = 0xffff_ffff;
const trackCount = (n: number) => Math.min(MAX_TRACK_COUNT, Math.max(0, n));

/** Convert the bounded server summary to the canvas track model used at broad zoom levels. */
export function summaryTracks(summary: Schemas["TimelineSummary"], nodeIndexes: ReadonlyMap<string, number>): TimelineTracks {
  const services = summary.services.flatMap((service) => {
    const node = nodeIndexes.get(service.service_id);
    if (node === undefined) return [];
    const counts = service.spans.map(trackCount);
    return [{ node, name: service.service_name, counts, spans_in_view: counts.reduce((sum, count) => sum + count, 0), clipped: false }];
  });
  return {
    from_ns: summary.from_ns,
    to_ns: summary.to_ns,
    buckets: summary.buckets,
    requests: summary.signals.map((bucket) => trackCount(bucket.requests)),
    errors: summary.signals.map((bucket) => trackCount(bucket.errors + bucket.error_logs)),
    events: summary.signals.map((bucket) => trackCount(bucket.events)),
    latency_p95_ms: summary.signals.map((bucket) => bucket.p95_ms ?? null),
    latency_samples: summary.signals.map((bucket) => trackCount(bucket.requests)),
    services,
    clips: [],
    markers: [],
    markers_total: 0,
    prev_item_ns: null,
    next_item_ns: null,
  };
}
