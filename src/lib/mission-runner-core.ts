import { appendFile, readFile, writeFile, rename } from "fs/promises";
import { join } from "path";
import {
  getMissionPath,
  type MissionPhase,
  type MissionState,
} from "./mission";
import {
  notifyCompleted,
  notifyCheckpoint,
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
 * Runs in a dedicated child process (agent-worker.ts) to keep the
 * Next.js UI server responsive. Communicates with the parent via
 * filesystem (mission.json, agent.log, kb/) and IPC (directives).
 */
export class MissionRunner {
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
