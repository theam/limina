import { readFile, writeFile, mkdir, readdir, rename, copyFile } from "fs/promises";
import { join, resolve } from "path";
import { nanoid } from "nanoid";
import { existsSync } from "fs";
import { execSync } from "child_process";

// Mission state machine
// CREATED → PLANNING → PLAN_READY → RUNNING → COMPLETED
//                                     │   ↑
//                                     ├→ CHECKPOINT_WAITING → RUNNING
//                                     ├→ ESCALATION_WAITING → RUNNING
//                                     ├→ STALLED
//                                     ├→ FAILED_RECOVERABLE → RUNNING (relaunch)
//                                     └→ KILLED
export type MissionPhase =
  | "CREATED"
  | "PLANNING"
  | "PLAN_READY"
  | "RUNNING"
  | "CHECKPOINT_WAITING"
  | "ESCALATION_WAITING"
  | "STALLED"
  | "FAILED_RECOVERABLE"
  | "COMPLETED"
  | "KILLED";

export type TemplateType = "search-relevance" | "system-investigation";

export interface MissionConfig {
  objective: string;
  context: string;
  repository?: string;
  successMetric: string;
  autonomyLevel: "full" | "checkpoint";
  maxRuntime: string;
  slackWebhook?: string;
  template: TemplateType;
}

export interface MissionState {
  id: string;
  phase: MissionPhase;
  pid?: number;
  pgid?: number;
  sessionId?: string;
  createdAt: string;
  planApprovedAt?: string;
  lastArtifactAt?: string;
  completedAt?: string;
  template: TemplateType;
  autonomyLevel: "full" | "checkpoint";
  slackWebhook?: string;
  objective: string;
  shareToken?: string;
  estimatedCost?: string;
  estimatedDuration?: string;
  failureReason?: string;
}

const MISSIONS_DIR = join(process.cwd(), "missions");

/**
 * Validate that a mission phase transition is legal.
 */
export function isValidTransition(
  from: MissionPhase,
  to: MissionPhase
): boolean {
  const validTransitions: Record<MissionPhase, MissionPhase[]> = {
    CREATED: ["PLANNING"],
    PLANNING: ["PLAN_READY", "FAILED_RECOVERABLE"],
    PLAN_READY: ["RUNNING"],
    RUNNING: [
      "COMPLETED",
      "CHECKPOINT_WAITING",
      "ESCALATION_WAITING",
      "STALLED",
      "FAILED_RECOVERABLE",
      "KILLED",
    ],
    CHECKPOINT_WAITING: ["RUNNING", "KILLED"],
    ESCALATION_WAITING: ["RUNNING", "KILLED"],
    STALLED: ["RUNNING", "KILLED"],
    FAILED_RECOVERABLE: ["RUNNING", "KILLED"],
    COMPLETED: [],
    KILLED: [],
  };

  return validTransitions[from]?.includes(to) ?? false;
}

/**
 * Generate CHALLENGE.md content from the intake form data.
 * Uses template strings — the agent is an LLM, so structured input is better than prose.
 */
export function generateChallengeMd(config: MissionConfig): string {
  const templateIntro =
    config.template === "search-relevance"
      ? "This is a search/relevance improvement mission."
      : "This is a system investigation mission.";

  return `# Research Mission

${templateIntro}

## Objective

${config.objective}

## Context & Baseline

${config.context}
${config.repository ? `\nRepository: ${config.repository}` : ""}

## Success Metric

${config.successMetric}

## Constraints

- Autonomy level: ${config.autonomyLevel === "full" ? "Fully autonomous — escalate only when truly blocked" : "Checkpoint mode — pause after each phase transition for approval"}
- Maximum runtime: ${config.maxRuntime}
- If evaluation data does not exist, generate it yourself and document how it was created.
- If additional tools, budget, or access are needed, ask with a clear justification via CEO_REQUESTS.md.

## Escalation Rules

When blocked on resources, access, or decisions, create an entry in \`kb/mission/CEO_REQUESTS.md\` with status PENDING. The system will notify the human operator via Slack. Do not proceed on blocked items — wait for a response.
`;
}

/**
 * Generate a minimal BACKLOG.md seed for the planning phase.
 * The agent will populate this with actual tasks during planning.
 */
function generateInitialBacklog(): string {
  return `# Research Backlog

## Last IDs
> T: T000

## Tasks

| ID | Title | Status | Priority | Type |
|---|---|---|---|---|

_The research agent will populate this backlog during the planning phase._
`;
}

/**
 * Scaffold the KB directory structure for a new mission.
 * Matches the Limina framework's expected layout.
 */
async function scaffoldKB(missionPath: string): Promise<void> {
  const kbPath = join(missionPath, "kb");
  const dirs = [
    "mission",
    "tasks",
    "research/hypotheses",
    "research/experiments",
    "research/findings",
    "research/literature",
    "research/data",
    "engineering/features",
    "engineering/investigations",
    "engineering/implementations",
    "engineering/retrospectives",
    "reports",
  ];

  for (const dir of dirs) {
    await mkdir(join(kbPath, dir), { recursive: true });
  }

  // Create INDEX.md
  await writeFile(
    join(kbPath, "INDEX.md"),
    "# Knowledge Base Index\n\n_Auto-populated as research progresses._\n"
  );

  // Create DECISIONS.md
  await writeFile(
    join(kbPath, "mission/DECISIONS.md"),
    "# Decisions\n\n_Decisions will be recorded here as research progresses._\n"
  );

  // Create CEO_REQUESTS.md
  await writeFile(
    join(kbPath, "mission/CEO_REQUESTS.md"),
    "# CEO Requests\n\n_Requests for human input will appear here._\n"
  );

  // Create DIRECTIVES.md
  await writeFile(
    join(kbPath, "mission/DIRECTIVES.md"),
    "# CEO Directives\n\n_Strategic instructions from the CEO to incorporate into ongoing work._\n"
  );

  // Copy framework files (CLAUDE.md, LIMINA.md, templates)
  const frameworkDir = join(process.cwd(), "src/lib/framework-files");
  if (existsSync(frameworkDir)) {
    await copyFile(join(frameworkDir, "CLAUDE.md"), join(missionPath, "CLAUDE.md"));
    await copyFile(join(frameworkDir, "LIMINA.md"), join(missionPath, "LIMINA.md"));
    // Copy templates
    const templateDir = join(missionPath, "templates");
    await mkdir(templateDir, { recursive: true });
    for (const t of ["task", "hypothesis", "experiment", "finding"]) {
      const src = join(frameworkDir, `template-${t}.md`);
      if (existsSync(src)) {
        await copyFile(src, join(templateDir, `${t}.md`));
      }
    }
  }

  // Init git repo
  try {
    execSync("git init && git add -A && git commit -m 'Mission initialized'", {
      cwd: missionPath,
      stdio: "ignore",
    });
  } catch {
    // Git init may fail if git is not configured — non-fatal
  }
}

/**
 * Estimate cost and duration based on template type.
 * Rough heuristics — will improve with real mission data.
 */
function estimateMission(template: TemplateType): {
  cost: string;
  duration: string;
} {
  if (template === "search-relevance") {
    return { cost: "$30–$150", duration: "12–48 hours" };
  }
  return { cost: "$20–$100", duration: "8–36 hours" };
}

/**
 * Create a new mission. Generates ID, scaffolds KB, writes CHALLENGE.md.
 */
export async function createMission(
  config: MissionConfig
): Promise<MissionState> {
  const id = `m_${nanoid(10)}`;
  const missionPath = join(MISSIONS_DIR, id);
  const estimate = estimateMission(config.template);

  await mkdir(missionPath, { recursive: true });
  await scaffoldKB(missionPath);

  // Write CHALLENGE.md
  const challengeContent = generateChallengeMd(config);
  await writeFile(
    join(missionPath, "kb/mission/CHALLENGE.md"),
    challengeContent
  );

  // Write initial BACKLOG.md
  await writeFile(
    join(missionPath, "kb/mission/BACKLOG.md"),
    generateInitialBacklog()
  );

  // Create mission state
  const state: MissionState = {
    id,
    phase: "CREATED",
    createdAt: new Date().toISOString(),
    template: config.template,
    autonomyLevel: config.autonomyLevel,
    slackWebhook: config.slackWebhook,
    objective: config.objective,
    estimatedCost: estimate.cost,
    estimatedDuration: estimate.duration,
  };

  await writeMissionState(id, state);
  return state;
}

/**
 * Validate mission ID format to prevent path traversal.
 */
function validateMissionId(id: string): void {
  if (!/^m_[a-zA-Z0-9_-]+$/.test(id)) {
    throw new Error(`Invalid mission ID: ${id}`);
  }
}

/**
 * Read mission state from mission.json.
 */
export async function readMissionState(
  missionId: string
): Promise<MissionState | null> {
  validateMissionId(missionId);
  const statePath = join(MISSIONS_DIR, missionId, "mission.json");
  try {
    const content = await readFile(statePath, "utf-8");
    return JSON.parse(content) as MissionState;
  } catch {
    return null;
  }
}

/**
 * Write mission state atomically (temp file + rename).
 */
export async function writeMissionState(
  missionId: string,
  state: MissionState
): Promise<void> {
  const statePath = join(MISSIONS_DIR, missionId, "mission.json");
  const tmpPath = `${statePath}.tmp.${Date.now()}`;
  await writeFile(tmpPath, JSON.stringify(state, null, 2));
  await rename(tmpPath, statePath);
}

/**
 * Update mission state fields without a phase transition.
 * Used for saving sessionId, timestamps, etc.
 */
export async function updateMissionState(
  missionId: string,
  updates: Partial<MissionState>
): Promise<MissionState> {
  const state = await readMissionState(missionId);
  if (!state) {
    throw new Error(`Mission ${missionId} not found`);
  }

  const updated: MissionState = { ...state, ...updates };
  await writeMissionState(missionId, updated);
  return updated;
}

/**
 * Transition mission to a new phase with validation.
 */
export async function transitionMission(
  missionId: string,
  newPhase: MissionPhase,
  updates?: Partial<MissionState>
): Promise<MissionState> {
  const state = await readMissionState(missionId);
  if (!state) {
    throw new Error(`Mission ${missionId} not found`);
  }

  if (!isValidTransition(state.phase, newPhase)) {
    throw new Error(
      `Invalid transition: ${state.phase} → ${newPhase} for mission ${missionId}`
    );
  }

  const updated: MissionState = {
    ...state,
    ...updates,
    phase: newPhase,
  };

  await writeMissionState(missionId, updated);
  return updated;
}

/**
 * List all missions with their current state.
 */
export async function listMissions(): Promise<MissionState[]> {
  const missions: MissionState[] = [];

  try {
    const dirs = await readdir(MISSIONS_DIR);
    for (const dir of dirs) {
      if (!dir.startsWith("m_")) continue;
      const state = await readMissionState(dir);
      if (state) {
        missions.push(state);
      }
    }
  } catch {
    // missions/ directory doesn't exist yet
  }

  // Sort by creation date, newest first
  return missions.sort(
    (a, b) => new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime()
  );
}

/**
 * Get the KB path for a mission.
 */
export function getMissionKBPath(missionId: string): string {
  return join(MISSIONS_DIR, missionId, "kb");
}

/**
 * Get the mission directory path.
 */
export function getMissionPath(missionId: string): string {
  return join(MISSIONS_DIR, missionId);
}

/**
 * Generate a share token for public report access.
 */
export async function generateShareToken(
  missionId: string
): Promise<string> {
  const state = await readMissionState(missionId);
  if (!state) throw new Error(`Mission ${missionId} not found`);
  if (state.phase !== "COMPLETED")
    throw new Error("Can only share completed missions");

  const token = nanoid(21);
  await writeMissionState(missionId, { ...state, shareToken: token });
  return token;
}

/**
 * Find a mission by its share token.
 */
export async function findMissionByShareToken(
  token: string
): Promise<MissionState | null> {
  const missions = await listMissions();
  return missions.find((m) => m.shareToken === token) ?? null;
}

/**
 * Atomic write to CEO_REQUESTS.md — handles concurrent access.
 */
export async function respondToCeoRequest(
  missionId: string,
  requestId: string,
  response: string
): Promise<void> {
  const reqPath = join(
    MISSIONS_DIR,
    missionId,
    "kb/mission/CEO_REQUESTS.md"
  );

  // Validate requestId format to prevent regex injection
  if (!/^REQ-\d+$/.test(requestId)) {
    throw new Error(`Invalid request ID format: ${requestId}`);
  }

  const content = await readFile(reqPath, "utf-8");

  // Find the request section and update its status
  // requestId is validated above, safe for regex. Response uses $$ escaping.
  const escapedResponse = response.replace(/\$/g, "$$$$");
  const updated = content.replace(
    new RegExp(
      `(## ${requestId}:.*?\\n>\\s+\\*\\*Status\\*\\*:\\s*)PENDING`,
      "s"
    ),
    `$1RESOLVED\n> **Response**: ${escapedResponse}\n> **Resolved at**: ${new Date().toISOString()}`
  );

  if (updated === content) {
    throw new Error(`CEO request ${requestId} not found or already resolved`);
  }

  // Atomic write
  const tmpPath = `${reqPath}.tmp.${Date.now()}`;
  await writeFile(tmpPath, updated);
  await rename(tmpPath, reqPath);
}
