export const METRIC_BAR_SEGMENTS = 12;
export const METRIC_BAR_COLORS = {
  healthy: 0x22c55e,
  warning: 0xf5b942,
  critical: 0xef4444,
  empty: 0x334155,
} as const;

export interface MetricBar {
  filled: number;
  segments: number;
  color: number | null;
}

export function metricBar(ratio: number | null): MetricBar {
  if (ratio === null || !Number.isFinite(ratio) || ratio < 0) {
    return { filled: 0, segments: METRIC_BAR_SEGMENTS, color: null };
  }
  const clamped = Math.min(1, ratio);
  const color = clamped >= 0.9
    ? METRIC_BAR_COLORS.critical
    : clamped >= 0.7
      ? METRIC_BAR_COLORS.warning
      : METRIC_BAR_COLORS.healthy;
  return {
    filled: Math.round(clamped * METRIC_BAR_SEGMENTS),
    segments: METRIC_BAR_SEGMENTS,
    color,
  };
}
