import { describe, it, expect } from "vitest";
import {
  generateChallengeMd,
  isValidTransition,
  type MissionConfig,
  type MissionPhase,
} from "../../src/lib/mission";

describe("generateChallengeMd", () => {
  const baseConfig: MissionConfig = {
    objective: "Improve search relevance for negation queries",
    context:
      "Current system uses BM25 + vector reranking. Negation queries fail.",
    repository: "github.com/acme/search-service",
    successMetric: "Recall@10 improvement >= 5%",
    autonomyLevel: "full",
    maxRuntime: "48h",
    template: "search-relevance",
  };

  it("generates structured CHALLENGE.md with all fields", () => {
    const md = generateChallengeMd(baseConfig);

    expect(md).toContain("## Objective");
    expect(md).toContain("Improve search relevance for negation queries");
    expect(md).toContain("## Context & Baseline");
    expect(md).toContain("BM25 + vector reranking");
    expect(md).toContain("Repository: github.com/acme/search-service");
    expect(md).toContain("## Success Metric");
    expect(md).toContain("Recall@10 improvement >= 5%");
    expect(md).toContain("## Constraints");
    expect(md).toContain("Fully autonomous");
    expect(md).toContain("Maximum runtime: 48h");
    expect(md).toContain("## Escalation Rules");
    expect(md).toContain("CEO_REQUESTS.md");
  });

  it("includes template-specific intro for search-relevance", () => {
    const md = generateChallengeMd(baseConfig);
    expect(md).toContain("search/relevance improvement mission");
  });

  it("includes template-specific intro for system-investigation", () => {
    const md = generateChallengeMd({
      ...baseConfig,
      template: "system-investigation",
    });
    expect(md).toContain("system investigation mission");
  });

  it("handles checkpoint autonomy level", () => {
    const md = generateChallengeMd({
      ...baseConfig,
      autonomyLevel: "checkpoint",
    });
    expect(md).toContain("Checkpoint mode");
  });

  it("omits repository when not provided", () => {
    const md = generateChallengeMd({
      ...baseConfig,
      repository: undefined,
    });
    expect(md).not.toContain("Repository:");
  });
});

describe("isValidTransition", () => {
  const validTransitions: Array<[MissionPhase, MissionPhase]> = [
    ["CREATED", "PLANNING"],
    ["PLANNING", "PLAN_READY"],
    ["PLANNING", "FAILED_RECOVERABLE"],
    ["PLAN_READY", "RUNNING"],
    ["RUNNING", "COMPLETED"],
    ["RUNNING", "CHECKPOINT_WAITING"],
    ["RUNNING", "ESCALATION_WAITING"],
    ["RUNNING", "STALLED"],
    ["RUNNING", "FAILED_RECOVERABLE"],
    ["RUNNING", "KILLED"],
    ["CHECKPOINT_WAITING", "RUNNING"],
    ["CHECKPOINT_WAITING", "KILLED"],
    ["ESCALATION_WAITING", "RUNNING"],
    ["ESCALATION_WAITING", "KILLED"],
    ["STALLED", "RUNNING"],
    ["STALLED", "KILLED"],
    ["FAILED_RECOVERABLE", "RUNNING"],
    ["FAILED_RECOVERABLE", "KILLED"],
  ];

  it.each(validTransitions)(
    "allows %s → %s",
    (from, to) => {
      expect(isValidTransition(from, to)).toBe(true);
    }
  );

  const invalidTransitions: Array<[MissionPhase, MissionPhase]> = [
    ["COMPLETED", "RUNNING"],
    ["KILLED", "RUNNING"],
    ["CREATED", "RUNNING"],
    ["PLAN_READY", "COMPLETED"],
    ["RUNNING", "PLANNING"],
    ["CREATED", "COMPLETED"],
  ];

  it.each(invalidTransitions)(
    "rejects %s → %s",
    (from, to) => {
      expect(isValidTransition(from, to)).toBe(false);
    }
  );
});
