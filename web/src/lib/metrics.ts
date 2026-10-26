import type { MetricInput } from "./types";

/** A measured outcome is not an activity score. Unknown evidence stays unknown. */
export function metricAssessment(metric: MetricInput): { label: string; met: boolean; progress: number | null } {
  const { current, baseline, target, direction } = metric;
  if (current == null || !Number.isFinite(current) || !metric.evidence_id) {
    return { label: "Not measured", met: false, progress: null };
  }
  const met = direction === "at_least" ? current >= target : current <= target;
  const meaningfulBaseline = baseline != null && Number.isFinite(baseline) &&
    (direction === "at_least" ? target > baseline : target < baseline);
  const progress = meaningfulBaseline ? Math.max(0, Math.min(100, ((current - baseline) / (target - baseline)) * 100)) : null;
  return { label: met ? "Target met" : direction === "at_least" ? "Below target" : "Above target", met, progress };
}

const number = new Intl.NumberFormat(undefined, { maximumFractionDigits: 4 });
export const metricValue = (value: number | null, unit: string) => value == null ? "Not measured" : `${number.format(value)}${unit === "%" ? "%" : unit ? ` ${unit}` : ""}`;
