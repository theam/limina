import { describe, expect, it } from "vitest";
import { metricAssessment } from "./metrics";
import type { MetricInput } from "./types";
const metric: MetricInput = { name: "Precision", criterion: "Held-out quality", unit: "%", direction: "at_least", baseline: 70, current: 85, target: 100, evidence_id: "F001", observed_at: "2026-10-21T12:00:00Z", notes: "" };
describe("evidence-backed KPI progress", () => {
  it("keeps an unmeasured value unknown rather than zero", () => {
    expect(metricAssessment({ ...metric, current: null })).toEqual({ label: "Not measured", met: false, progress: null });
    expect(metricAssessment({ ...metric, evidence_id: null }).progress).toBeNull();
  });
  it("measures distance from baseline to target rather than raw activity", () => {
    expect(metricAssessment(metric)).toEqual({ label: "Below target", met: false, progress: 50 });
    expect(metricAssessment({ ...metric, current: 110 })).toEqual({ label: "Target met", met: true, progress: 100 });
    expect(metricAssessment({ ...metric, current: 60 }).progress).toBe(0);
  });
  it("supports lower-is-better KPIs and zero targets", () => {
    expect(metricAssessment({ ...metric, direction: "at_most", baseline: 20, current: 10, target: 0 }).progress).toBe(50);
    expect(metricAssessment({ ...metric, direction: "at_most", baseline: 20, current: 0, target: 0 }).met).toBe(true);
  });
  it("does not invent a progress percentage without a meaningful baseline", () => {
    expect(metricAssessment({ ...metric, baseline: null }).progress).toBeNull();
    expect(metricAssessment({ ...metric, baseline: 100 }).progress).toBeNull();
    expect(metricAssessment({ ...metric, baseline: 110 }).progress).toBeNull();
  });
});
