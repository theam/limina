import { describe, it, expect } from "vitest";
import {
  parseMetadata,
  parseTitle,
  parseIdFromFilename,
  detectArtifactType,
  parseArtifact,
  parseBacklog,
  parseCeoRequests,
  inferPhase,
  type Artifact,
} from "../../src/lib/kb-parser";

describe("parseMetadata", () => {
  it("extracts key-value pairs from blockquote format", () => {
    const content = `# H001 — Test
> **Status**: PROPOSED
> **Task**: T001
> **Created**: 2026-03-23`;

    const metadata = parseMetadata(content);
    expect(metadata.status).toBe("PROPOSED");
    expect(metadata.task).toBe("T001");
    expect(metadata.created).toBe("2026-03-23");
  });

  it("handles missing metadata gracefully", () => {
    const metadata = parseMetadata("# Title\n\nNo metadata here.");
    expect(metadata).toEqual({});
  });

  it("handles extra whitespace in values", () => {
    const content = "> **Status**:   CONFIRMED   ";
    const metadata = parseMetadata(content);
    expect(metadata.status).toBe("CONFIRMED");
  });
});

describe("parseTitle", () => {
  it("extracts title from H1 heading with artifact ID", () => {
    expect(parseTitle("# H001 — Negation handling")).toBe(
      "Negation handling"
    );
  });

  it("handles em dash, en dash, and hyphen separators", () => {
    expect(parseTitle("# T003 – Task title")).toBe("Task title");
    expect(parseTitle("# E002 - Experiment title")).toBe("Experiment title");
  });

  it("handles multi-prefix IDs", () => {
    expect(parseTitle("# FT001 — Feature spec")).toBe("Feature spec");
    expect(parseTitle("# INV002 — Investigation")).toBe("Investigation");
  });

  it("returns Untitled for missing heading", () => {
    expect(parseTitle("No heading here")).toBe("Untitled");
  });

  it("returns title without ID prefix when no ID present", () => {
    expect(parseTitle("# Just a plain title")).toBe("Just a plain title");
  });
});

describe("parseIdFromFilename", () => {
  it("extracts ID from standard filenames", () => {
    expect(parseIdFromFilename("H001-negation-handling.md")).toBe("H001");
    expect(parseIdFromFilename("T003-investigate.md")).toBe("T003");
    expect(parseIdFromFilename("FT001-feature.md")).toBe("FT001");
    expect(parseIdFromFilename("CR002-review.md")).toBe("CR002");
  });

  it("returns empty string for non-artifact files", () => {
    expect(parseIdFromFilename("README.md")).toBe("");
    expect(parseIdFromFilename("INDEX.md")).toBe("");
  });
});

describe("detectArtifactType", () => {
  it("detects all artifact types from file paths", () => {
    expect(detectArtifactType("research/hypotheses/H001-test.md")).toBe(
      "hypothesis"
    );
    expect(detectArtifactType("research/experiments/E001-test.md")).toBe(
      "experiment"
    );
    expect(detectArtifactType("research/findings/F001-test.md")).toBe(
      "finding"
    );
    expect(detectArtifactType("research/literature/L001-test.md")).toBe(
      "literature"
    );
    expect(detectArtifactType("reports/CR001-review.md")).toBe(
      "challenge_review"
    );
    expect(detectArtifactType("reports/SR001-review.md")).toBe(
      "strategic_review"
    );
    expect(detectArtifactType("tasks/T001-task.md")).toBe("task");
  });

  it("returns null for non-artifact paths", () => {
    expect(detectArtifactType("mission/CHALLENGE.md")).toBeNull();
    expect(detectArtifactType("INDEX.md")).toBeNull();
  });
});

describe("parseArtifact", () => {
  it("parses a complete hypothesis file", () => {
    const content = `# H001 — Query rewriting can decompose negation

> **Status**: CONFIRMED
> **Task**: T001
> **Created**: 2026-03-23

## Statement

If we rewrite negation queries, then recall improves.`;

    const artifact = parseArtifact(
      content,
      "research/hypotheses/H001-query-rewriting.md"
    );
    expect(artifact).not.toBeNull();
    expect(artifact!.type).toBe("hypothesis");
    expect(artifact!.id).toBe("H001");
    expect(artifact!.title).toBe(
      "Query rewriting can decompose negation"
    );
    expect(artifact!.status).toBe("CONFIRMED");
  });

  it("returns null for non-artifact paths", () => {
    const artifact = parseArtifact("# README", "README.md");
    expect(artifact).toBeNull();
  });
});

describe("parseBacklog", () => {
  it("extracts task entries from markdown table", () => {
    const content = `# Backlog

| ID | Title | Status | Priority | Type |
|---|---|---|---|---|
| T001 | Analyze failure modes | IN_PROGRESS | P0 | research |
| T002 | Generate evaluation dataset | TODO | P0 | research |
| T003 | Test query rewriting | BACKLOG | P1 | research |`;

    const tasks = parseBacklog(content);
    expect(tasks).toHaveLength(3);
    expect(tasks[0]).toEqual({
      id: "T001",
      title: "Analyze failure modes",
      status: "IN_PROGRESS",
      priority: "P0",
      type: "research",
    });
  });

  it("returns empty array for empty backlog", () => {
    const tasks = parseBacklog("# Backlog\n\nNo tasks yet.");
    expect(tasks).toEqual([]);
  });
});

describe("parseCeoRequests", () => {
  it("extracts pending and resolved requests", () => {
    const content = `# CEO Requests

## REQ-001: Need access to production query logs
> **Status**: PENDING
> **Date**: 2026-03-23

We need query logs for the last 30 days to build the evaluation set.

## REQ-002: Budget approval for embedding training
> **Status**: RESOLVED
> **Date**: 2026-03-23
> **Response**: Approved, use the staging GPU cluster.

Need 4 GPU-hours for contrastive fine-tuning.`;

    const requests = parseCeoRequests(content);
    expect(requests).toHaveLength(2);
    expect(requests[0].id).toBe("REQ-001");
    expect(requests[0].status).toBe("PENDING");
    expect(requests[0].title).toBe(
      "Need access to production query logs"
    );
    expect(requests[1].status).toBe("RESOLVED");
  });

  it("returns empty array when no requests exist", () => {
    const requests = parseCeoRequests(
      "# CEO Requests\n\n_No requests yet._"
    );
    expect(requests).toEqual([]);
  });
});

describe("inferPhase", () => {
  it("returns idle for no artifacts", () => {
    expect(inferPhase([])).toBe("idle");
  });

  it("infers phase from latest artifact type", () => {
    const artifacts: Artifact[] = [
      {
        type: "hypothesis",
        id: "H001",
        title: "Test",
        status: "PROPOSED",
        metadata: {},
        content: "",
        filePath: "",
      },
      {
        type: "experiment",
        id: "E001",
        title: "Test",
        status: "RUNNING",
        metadata: {},
        content: "",
        filePath: "",
      },
    ];
    // E001 > H001 alphabetically, so experiment is latest
    expect(inferPhase(artifacts)).toBe("experimenting");
  });

  it("detects reviewing phase from challenge review", () => {
    const artifacts: Artifact[] = [
      {
        type: "challenge_review",
        id: "CR001",
        title: "Review",
        status: "COMPLETE",
        metadata: {},
        content: "",
        filePath: "",
      },
    ];
    expect(inferPhase(artifacts)).toBe("reviewing");
  });
});
