import { writeFile, mkdir, rename } from "fs/promises";
import { join } from "path";

const MISSIONS_DIR = join(process.cwd(), "missions");

/**
 * Seed a mission directly on the filesystem for E2E tests.
 * This bypasses the API to set up specific states for testing.
 */
export async function seedMission(
  id: string,
  phase: string,
  objective: string = "Test mission objective for E2E testing of the Limina web interface"
): Promise<void> {
  const missionPath = join(MISSIONS_DIR, id);
  const kbPath = join(missionPath, "kb");

  // Create directory structure
  const dirs = [
    "mission",
    "tasks",
    "research/hypotheses",
    "research/experiments",
    "research/findings",
    "research/literature",
    "research/data",
    "reports",
  ];
  for (const dir of dirs) {
    await mkdir(join(kbPath, dir), { recursive: true });
  }

  // Write mission.json
  const state = {
    id,
    phase,
    createdAt: new Date().toISOString(),
    template: "search-relevance",
    autonomyLevel: "full",
    objective,
    estimatedCost: "$30–$150",
    estimatedDuration: "12–48 hours",
    ...(phase === "COMPLETED"
      ? { completedAt: new Date().toISOString() }
      : {}),
  };
  await writeFile(
    join(missionPath, "mission.json"),
    JSON.stringify(state, null, 2)
  );

  // Write CHALLENGE.md
  await writeFile(
    join(kbPath, "mission/CHALLENGE.md"),
    `# Research Mission\n\n## Objective\n\n${objective}\n`
  );

  // Write BACKLOG.md
  await writeFile(
    join(kbPath, "mission/BACKLOG.md"),
    `# Research Backlog\n\n## Last IDs\n> T: T000\n\n## Tasks\n\n| ID | Title | Status | Priority | Type |\n|---|---|---|---|---|\n`
  );

  // Write empty CEO_REQUESTS.md
  await writeFile(
    join(kbPath, "mission/CEO_REQUESTS.md"),
    "# CEO Requests\n\n_No requests yet._\n"
  );

  // Write INDEX.md
  await writeFile(join(kbPath, "INDEX.md"), "# Knowledge Base Index\n");
}

/**
 * Seed a mission with a populated plan (PLAN_READY state).
 */
export async function seedMissionWithPlan(id: string): Promise<void> {
  await seedMission(id, "PLAN_READY");

  const kbPath = join(MISSIONS_DIR, id, "kb");
  await writeFile(
    join(kbPath, "mission/BACKLOG.md"),
    `# Research Backlog

## Last IDs
> T: T002

## Tasks

| ID | Title | Status | Priority | Type |
|---|---|---|---|---|
| T001 | Analyze failure modes in current system | TODO | P0 | research |
| T002 | Test query rewriting approaches | TODO | P1 | research |
`
  );
}

/**
 * Seed a mission with artifacts (RUNNING state).
 */
export async function seedMissionWithArtifacts(id: string): Promise<void> {
  await seedMissionWithPlan(id);

  const missionPath = join(MISSIONS_DIR, id);
  const kbPath = join(missionPath, "kb");

  // Update state to RUNNING
  const stateContent = await readFileStr(join(missionPath, "mission.json"));
  const state = JSON.parse(stateContent);
  state.phase = "RUNNING";
  state.planApprovedAt = new Date().toISOString();
  await writeFile(
    join(missionPath, "mission.json"),
    JSON.stringify(state, null, 2)
  );

  // Add hypothesis
  await writeFile(
    join(kbPath, "research/hypotheses/H001-query-rewriting.md"),
    `# H001 — Query rewriting can decompose negation

> **Status**: CONFIRMED
> **Task**: T001
> **Created**: 2026-03-23

## Statement
If we rewrite negation queries, then recall improves.
`
  );

  // Add experiment
  await writeFile(
    join(kbPath, "research/experiments/E001-rewriting-test.md"),
    `# E001 — Testing query rewriting

> **Status**: COMPLETED
> **Task**: T001
> **Hypothesis**: H001
> **Created**: 2026-03-23

## Procedure
Test query rewriting with LLM decomposition.
`
  );

  // Add finding
  await writeFile(
    join(kbPath, "research/findings/F001-rewriting-results.md"),
    `# F001 — Query rewriting achieves +3.2% recall

> **Task**: T001
> **Hypothesis**: H001
> **Experiment**: E001
> **Impact**: HIGH
> **Created**: 2026-03-23

## Finding
Query rewriting with LLM decomposition achieves +3.2% recall improvement but adds 200ms latency.
`
  );
}

/**
 * Seed a completed mission with full report data.
 */
export async function seedCompletedMission(id: string): Promise<void> {
  await seedMissionWithArtifacts(id);

  const missionPath = join(MISSIONS_DIR, id);
  const stateContent = await readFileStr(join(missionPath, "mission.json"));
  const state = JSON.parse(stateContent);
  state.phase = "COMPLETED";
  state.completedAt = new Date().toISOString();
  await writeFile(
    join(missionPath, "mission.json"),
    JSON.stringify(state, null, 2)
  );
}

/**
 * Seed a mission with a pending CEO request.
 */
export async function seedMissionWithEscalation(id: string): Promise<void> {
  await seedMissionWithArtifacts(id);

  const kbPath = join(MISSIONS_DIR, id, "kb");

  // Update state to ESCALATION_WAITING
  const missionPath = join(MISSIONS_DIR, id);
  const stateContent = await readFileStr(join(missionPath, "mission.json"));
  const state = JSON.parse(stateContent);
  state.phase = "ESCALATION_WAITING";
  await writeFile(
    join(missionPath, "mission.json"),
    JSON.stringify(state, null, 2)
  );

  await writeFile(
    join(kbPath, "mission/CEO_REQUESTS.md"),
    `# CEO Requests

## REQ-001: Need access to production query logs
> **Status**: PENDING
> **Date**: 2026-03-23

We need query logs for the last 30 days to build the evaluation set.
`
  );
}

async function readFileStr(path: string): Promise<string> {
  const { readFile } = await import("fs/promises");
  return readFile(path, "utf-8");
}

/**
 * Clean up test missions.
 */
export async function cleanupMissions(): Promise<void> {
  const { rm } = await import("fs/promises");
  const { existsSync } = await import("fs");
  if (existsSync(MISSIONS_DIR)) {
    await rm(MISSIONS_DIR, { recursive: true, force: true });
  }
  await mkdir(MISSIONS_DIR, { recursive: true });
}
