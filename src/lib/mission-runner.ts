import { existsSync } from "fs";
import { appendFile, readFile, writeFile, rename } from "fs/promises";
import { join } from "path";
import {
  getMissionPath,
  transitionMission,
  type MissionPhase,
  type MissionState,
} from "./mission";
import { readKBState } from "./kb-parser";
import {
  notifyCompleted,
  notifyCheckpoint,
  notifyStalled,
  notifyFailed,
} from "./notify";

// Lazy import to avoid crashing if SDK not installed yet
let sdkQuery: typeof import("@anthropic-ai/claude-agent-sdk").query | null =
  null;

async function getQuery() {
  if (!sdkQuery) {
    const sdk = await import("@anthropic-ai/claude-agent-sdk");
    sdkQuery = sdk.query;
  }
  return sdkQuery;
}

/**
 * MissionRunner — manages a research mission using the Claude Agent SDK.
 *
 * Manages a research mission using the Claude Agent SDK,
 * enabling mid-session directive injection via session continuity.
 */
class MissionRunner {
  private sessionId: string | null = null;
  private abortController: AbortController | null = null;
  private pendingDirectives: string[] = [];
  private running = false;
  private missionId: string;
  private missionPath: string;
  private logPath: string;
  private budgetUsd: number | undefined;

  constructor(missionId: string, missionPath?: string) {
    this.missionId = missionId;
    this.missionPath = missionPath || getMissionPath(missionId);
    this.logPath = join(this.missionPath, "agent.log");
  }

  /**
   * Parse budget from mission state string (e.g. "$100", "50", "100 USD").
   */
  private parseBudget(state: { estimatedCost?: string }): number | undefined {
    const budgetStr = state.estimatedCost || "";
    const match = budgetStr.match(/([\d.]+)/);
    return match ? parseFloat(match[1]) : undefined;
  }

  /**
   * Read mission.json from this mission's directory.
   */
  private async readState(): Promise<MissionState | null> {
    try {
      const raw = await readFile(join(this.missionPath, "mission.json"), "utf-8");
      return JSON.parse(raw);
    } catch {
      return null;
    }
  }

  /**
   * Write mission.json atomically to this mission's directory.
   */
  private async writeState(state: MissionState): Promise<void> {
    const statePath = join(this.missionPath, "mission.json");
    const tmp = `${statePath}.tmp.${Date.now()}`;
    await writeFile(tmp, JSON.stringify(state, null, 2));
    await rename(tmp, statePath);
  }

  /**
   * Update specific fields in mission.json.
   */
  private async updateState(updates: Partial<MissionState>): Promise<void> {
    const state = await this.readState();
    if (!state) return;
    Object.assign(state, updates);
    await this.writeState(state);
  }

  /**
   * Transition mission phase with the update written to disk.
   */
  private async transition(newPhase: MissionPhase, extra?: Partial<MissionState>): Promise<void> {
    await this.updateState({ phase: newPhase, ...extra });
  }

  /**
   * Run a mission phase using the Claude Agent SDK.
   *
   * Phase determines behavior:
   * - planning: single turn to generate BACKLOG.md, then PLAN_READY
   * - running: work-review loop until DONE or directive
   * - checkpoint-resume: single turn, then CHECKPOINT_WAITING
   */
  async run(phase: "planning" | "running" | "checkpoint-resume"): Promise<void> {
    if (this.running) {
      throw new Error("MissionRunner is already running");
    }

    this.running = true;
    const state = await this.readState();
    if (!state) throw new Error(`Mission ${this.missionId} not found`);

    // Resume session if one exists
    this.sessionId = state.sessionId || null;
    this.budgetUsd = this.parseBudget(state);

    // Transition to appropriate phase
    const newPhase: MissionPhase =
      phase === "planning" ? "PLANNING" : "RUNNING";
    await this.transition(newPhase);

    // Start stall detection
    startStallDetection(this.missionId, this.missionPath);

    try {
      if (phase === "planning") {
        await this.runSingleTurn(
          "Read CLAUDE.md and follow the session protocol. Generate a research plan: create tasks in kb/tasks/ and populate kb/mission/BACKLOG.md. Do NOT start executing research — only plan."
        );
        await this.transition("PLAN_READY");
      } else if (phase === "checkpoint-resume") {
        await this.runSingleTurn(
          "Continue research. Read CLAUDE.md and follow the continuity protocol. Complete the current phase only, then stop."
        );
        await this.transition("CHECKPOINT_WAITING");
        if (state.slackWebhook) {
          await notifyCheckpoint(state);
        }
      } else {
        // Running phase: work-review loop
        await this.runLoop(state);
      }
    } catch (err) {
      const reason =
        err instanceof Error ? err.message : "Unknown error";
      await this.transition("FAILED_RECOVERABLE", { failureReason: reason });
      const currentState = await this.readState();
      if (currentState?.slackWebhook) {
        await notifyFailed(currentState, reason);
      }
    } finally {
      this.running = false;
      this.abortController = null;
      stopStallDetection();
    }
  }

  /**
   * The main research loop: work → deliver directives → review → exit check.
   */
  private async runLoop(
    state: MissionState
  ): Promise<void> {
    let done = false;

    while (!done && this.running) {
      // 1. Build work prompt with any pending directives
      let prompt =
        "Continue research. Read CLAUDE.md and follow the continuity protocol.";
      if (this.pendingDirectives.length > 0) {
        prompt +=
          "\n\nCEO DIRECTIVES (incorporate immediately):\n" +
          this.pendingDirectives
            .map((d, i) => `${i + 1}. ${d}`)
            .join("\n");
        this.pendingDirectives = [];
      }

      // 2. Run work phase
      await this.runSingleTurn(prompt);

      if (!this.running) break; // Killed during work

      // 3. Deliver any directives that arrived during work
      if (this.pendingDirectives.length > 0) {
        const directivePrompt =
          "CEO DIRECTIVES (incorporate immediately):\n" +
          this.pendingDirectives
            .map((d, i) => `${i + 1}. ${d}`)
            .join("\n");
        this.pendingDirectives = [];
        await this.runSingleTurn(directivePrompt);
      }

      if (!this.running) break;

      // 4. Review phase
      const reviewOutput = await this.runSingleTurn(
        "Review current status and verify if we achieved the target mission. " +
          "If we achieved it, respond with DONE as the last word. " +
          "If not, respond with ITERATE as the last word."
      );

      // 5. Check exit condition
      done = this.parseExitCondition(reviewOutput);
    }

    if (done) {
      const currentState = await this.readState();
      await this.transition("COMPLETED", {
        completedAt: new Date().toISOString(),
      });
      if (currentState?.slackWebhook) {
        await notifyCompleted(currentState);
      }
    }
  }

  /**
   * Run a single SDK query turn and return the assistant's text output.
   */
  private async runSingleTurn(prompt: string): Promise<string> {
    const queryFn = await getQuery();
    this.abortController = new AbortController();

    const options: Record<string, unknown> = {
      cwd: this.missionPath,
      model: process.env.LIMINA_MODEL || "claude-opus-4-6",
      thinking: { type: "adaptive" },
      permissionMode: "bypassPermissions",
      allowDangerouslySkipPermissions: true,
      tools: { type: "preset", preset: "claude_code" },
      settingSources: ["project"],
      abortController: this.abortController,
      maxBudgetUsd: this.budgetUsd,
      env: {
        ...process.env,
        CLAUDE_AGENT_SDK_CLIENT_APP: "limina/1.0",
      },
    };

    // Resume session for continuity
    if (this.sessionId) {
      options.resume = this.sessionId;
    }

    let assistantText = "";

    for await (const msg of queryFn({ prompt, options: options as any })) {
      // Capture session ID from init message
      if (
        msg.type === "system" &&
        "subtype" in msg &&
        msg.subtype === "init"
      ) {
        this.sessionId = (msg as any).session_id;
        // Persist session ID for restart recovery
        await this.updateState({ sessionId: this.sessionId! });
      }

      // Collect assistant text
      if (msg.type === "assistant" && "message" in msg) {
        const content = (msg as any).message?.content;
        if (Array.isArray(content)) {
          for (const block of content) {
            if (block.type === "text") {
              assistantText += block.text;
            }
          }
        }
      }

      // Capture cost from result messages
      if (msg.type === "result" && "total_cost_usd" in msg) {
        await this.updateCost((msg as any).total_cost_usd);
      }

      // Stream to log file
      await this.appendToLog(msg);
    }

    return assistantText;
  }

  /**
   * Write cumulative cost to cost.json.
   * The SDK's total_cost_usd is the cumulative session spend.
   */
  private async updateCost(totalCostUsd: number): Promise<void> {
    try {
      const costPath = join(this.missionPath, "cost.json");
      let costState = { budget: this.budgetUsd ?? null, spent: 0, phases: [] as unknown[] };
      try {
        const raw = await readFile(costPath, "utf-8");
        costState = JSON.parse(raw);
      } catch {}
      costState.spent = totalCostUsd;
      const tmp = `${costPath}.tmp.${Date.now()}`;
      await writeFile(tmp, JSON.stringify(costState, null, 2));
      await rename(tmp, costPath);
    } catch {
      // Non-fatal
    }
  }

  /**
   * Parse review output for DONE/ITERATE exit condition.
   */
  private parseExitCondition(output: string): boolean {
    const normalized = output.trim().toUpperCase();
    // Check if the last word is DONE
    return normalized.endsWith("DONE");
  }

  /**
   * Append an SDK message to the agent log file.
   */
  private async appendToLog(msg: unknown): Promise<void> {
    try {
      const entry = `[${new Date().toISOString()}] ${JSON.stringify(msg)}\n`;
      await appendFile(this.logPath, entry);
    } catch {
      // Non-fatal: don't crash the mission if logging fails
    }
  }

  /**
   * Queue a directive for delivery at the next phase boundary.
   * Called from the /api/directive endpoint.
   */
  injectDirective(text: string): void {
    this.pendingDirectives.push(text);
  }

  /**
   * Stop the runner by aborting the current SDK query.
   */
  kill(): void {
    this.running = false;
    if (this.abortController) {
      this.abortController.abort();
    }
  }

  isRunning(): boolean {
    return this.running;
  }

  getSessionId(): string | null {
    return this.sessionId;
  }
}

// ─── Module-level state (single-tenant, one mission at a time) ───

let activeRunner: MissionRunner | null = null;
let activeMissionId: string | null = null;
let stallTimer: ReturnType<typeof setInterval> | null = null;
let lastArtifactTime: number = Date.now();

/**
 * Start a mission using the Claude Agent SDK.
 */
export async function startMission(
  missionId: string,
  phase: "planning" | "running" | "checkpoint-resume",
  missionDir?: string
): Promise<void> {
  if (activeRunner?.isRunning()) {
    throw new Error(
      `Another mission is already running: ${activeMissionId}`
    );
  }

  const missionPath = missionDir || getMissionPath(missionId);
  if (!existsSync(missionPath)) {
    throw new Error(`Mission directory not found: ${missionPath}`);
  }

  activeRunner = new MissionRunner(missionId, missionPath);
  activeMissionId = missionId;
  lastArtifactTime = Date.now();

  // Run in background — don't await (the loop runs until done/killed)
  activeRunner.run(phase).finally(() => {
    if (activeMissionId === missionId) {
      activeRunner = null;
      activeMissionId = null;
    }
  });
}

/**
 * Kill the active mission.
 */
export async function killMission(missionId: string): Promise<void> {
  if (activeMissionId === missionId && activeRunner) {
    activeRunner.kill();
    stopStallDetection();
    activeRunner = null;
    activeMissionId = null;
  }
}

/**
 * Get the active MissionRunner instance for directive injection.
 */
export function getMissionRunner(): MissionRunner | null {
  return activeRunner;
}

/**
 * Start stall detection — checks every 5 minutes if new artifacts appeared.
 */
function startStallDetection(missionId: string, missionPath: string): void {
  stopStallDetection();

  const STALL_THRESHOLD_MS = 30 * 60 * 1000; // 30 minutes
  const CHECK_INTERVAL_MS = 5 * 60 * 1000; // 5 minutes

  stallTimer = setInterval(async () => {
    if (activeMissionId !== missionId) {
      stopStallDetection();
      return;
    }

    const kbPath = join(missionPath, "kb");
    try {
      const kbState = await readKBState(kbPath);
      const totalArtifacts =
        kbState.artifactCounts.hypotheses +
        kbState.artifactCounts.experiments +
        kbState.artifactCounts.findings;

      const now = Date.now();
      if (now - lastArtifactTime > STALL_THRESHOLD_MS && totalArtifacts > 0) {
        // Read state to check for slack webhook
        try {
          const raw = await readFile(join(missionPath, "mission.json"), "utf-8");
          const state = JSON.parse(raw) as MissionState;
          if (state.slackWebhook) {
            await notifyStalled(state);
          }
        } catch {}
      }
    } catch {
      // KB not readable yet
    }
  }, CHECK_INTERVAL_MS);
}

function stopStallDetection(): void {
  if (stallTimer) {
    clearInterval(stallTimer);
    stallTimer = null;
  }
}

/**
 * Update the last artifact timestamp. Called by the status endpoint
 * when it detects new artifacts.
 */
export function updateLastArtifactTime(): void {
  lastArtifactTime = Date.now();
}

/**
 * Get the currently active mission ID (if any).
 */
export function getActiveMissionId(): string | null {
  return activeMissionId;
}

/**
 * Check if the active runner is still working.
 */
export function isProcessAlive(): boolean {
  return activeRunner?.isRunning() ?? false;
}

/**
 * On server startup, check for missions that were running when the server stopped.
 * With session continuity, missions can potentially be resumed.
 */
export async function recoverFromRestart(): Promise<void> {
  const { listMissions } = await import("./mission");
  const missions = await listMissions();

  for (const mission of missions) {
    if (
      mission.phase === "RUNNING" ||
      mission.phase === "PLANNING" ||
      mission.phase === "CHECKPOINT_WAITING" ||
      mission.phase === "ESCALATION_WAITING" ||
      mission.phase === "STALLED"
    ) {
      await transitionMission(mission.id, "FAILED_RECOVERABLE", {
        failureReason:
          "Server restarted while mission was running. Session can be resumed.",
      });
    }
  }
}

/**
 * Check if the Claude Agent SDK is available.
 */
export async function checkAgentSdkAvailable(): Promise<boolean> {
  try {
    await getQuery();
    return true;
  } catch {
    return false;
  }
}
