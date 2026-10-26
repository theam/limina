import { describe, expect, it } from "vitest";
import { buildVault, linkifyWikilinks, searchVault, splitFrontmatter } from "./vault";

const files: Record<string, string> = {
  "mission/CHALLENGE.md": `---
aliases: ["CHALLENGE"]
type: mission
challenge_id: "abc"
challenge_slug: "demo"
---

# Research Mission — Demo

## Objective

Find out whether [[H001]] holds.

## Context

_Not recorded._

## Links

- Active State: [[ACTIVE]]
- Dashboard: [[DASHBOARD]]
`,
  "ACTIVE.md": `---
aliases: ["ACTIVE"]
type: active-state
coordinator_version: 3
---

# Active State

## Current Objective

Test the cache.

## Links

- Mission: [[CHALLENGE]]
- Working Note: [[E001]]
`,
  "research/hypotheses/H001-cache-helps.md": `---
id: "H001"
aliases: ["H001"]
type: hypothesis
status: "TESTING"
created: "2026-09-01"
last_updated: "2026-09-02"
tags: []
---

# H001 — Caching reduces latency

> **Status**: TESTING
> **Created**: 2026-09-01
> **Last updated**: 2026-09-02

## Statement

A **warm cache** cuts p95 latency by 30%.

## Mechanism

_Not recorded._

## Links

- Mission: [[CHALLENGE]]
- Experiment: [[E001]]
`,
  "research/experiments/E001-load-test.md": `---
id: "E001"
aliases: ["E001"]
type: experiment
status: "RUNNING"
hypothesis: "H001"
created: "2026-09-02"
completed: ""
tags: []
---

# E001 — Load test

> **Status**: RUNNING
> **Hypothesis**: [[H001]]
> **Created**: 2026-09-02
> **Completed**: 

## Objective

Measure p95 with \`[[not-a-link]]\` in code.

## Progress

- [ ] No observations recorded.

## Links

- Mission: [[CHALLENGE]]
- Parent Hypothesis: [[H001]]
`,
};

describe("vault", () => {
  const vault = buildVault(files);

  it("parses frontmatter scalars and JSON arrays", () => {
    const { data } = splitFrontmatter(files["research/hypotheses/H001-cache-helps.md"]);
    expect(data.aliases).toEqual(["H001"]);
    expect(data.status).toBe("TESTING");
  });

  it("orders project notes before research notes and resolves aliases", () => {
    expect(vault.docs.map((doc) => doc.key)).toEqual(["CHALLENGE", "ACTIVE", "H001", "E001"]);
    expect(vault.resolve("h001")?.title).toBe("Caching reduces latency");
    expect(vault.resolve("E001-load-test")?.key).toBe("E001");
    expect(vault.resolve("CHALLENGE")?.title).toBe("Mission");
  });

  it("turns the metadata callout into properties and hides projection boilerplate", () => {
    const h = vault.resolve("H001")!;
    expect(h.properties).toMatchObject({ Status: "TESTING", Created: "2026-09-01" });
    expect(h.body).not.toMatch(/Status\*\*/);
    expect(h.body).not.toMatch(/## Links/);
    expect(h.pending).toEqual(["Mechanism"]);
    expect(h.outline.map((item) => item.text)).toEqual(["Statement"]);
    expect(h.excerpt).toBe("A warm cache cuts p95 latency by 30%.");
    const mission = vault.resolve("CHALLENGE")!;
    expect(mission.properties).not.toHaveProperty("Challenge id");
  });

  it("treats empty progress as pending and records declared connections", () => {
    const e = vault.resolve("E001")!;
    expect(e.pending).toContain("Progress");
    expect(e.properties.Hypothesis).toBe("[[H001]]");
    expect(e.connections).toContainEqual({ label: "Parent Hypothesis", target: "H001" });
  });

  it("computes backlinks from links and inline wikilinks", () => {
    expect(vault.backlinks("H001").map((doc) => doc.key)).toEqual(["CHALLENGE", "E001"]);
    expect(vault.backlinks("E001").map((doc) => doc.key)).toEqual(["ACTIVE", "H001"]);
  });

  it("rewrites wikilinks outside code only", () => {
    expect(linkifyWikilinks("See [[H001]] and [[E001|the test]] but `[[raw]]`.")).toBe(
      "See [H001](wiki:H001) and [the test](wiki:E001) but `[[raw]]`.",
    );
  });

  it("searches titles before bodies", () => {
    expect(searchVault(vault, "load")[0].key).toBe("E001");
    expect(searchVault(vault, "warm cache").map((doc) => doc.key)).toEqual(["H001"]);
  });
});
