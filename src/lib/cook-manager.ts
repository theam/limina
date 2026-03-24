import { spawn, type ChildProcess } from "child_process";
import { existsSync } from "fs";
import { appendFile, readFile } from "fs/promises";
import { join } from "path";
import {
  readMissionState,
  transitionMission,
  getMissionPath,
  getMissionKBPath,
  type MissionPhase,
} from "./mission";
import { readKBState } from "./kb-parser";
import {
  notifyCompleted,
  notifyEscalation,
  notifyCheckpoint,
  notifyStalled,
  notifyFailed,
} from "./notify";

// In-memory process tracking (single-tenant, one mission at a time)
let activeProcess: ChildProcess | null = null;
let activeMissionId: string | null = null;
let stallTimer: ReturnType<typeof setInterval> | null = null;
let lastArtifactTime: number = Date.now();

/**
 * Check if cook CLI is installed and accessible.
 */
export async function checkCookInstalled(): Promise<boolean> {
  try {
    const { execSync } = await import("child_process");
    execSync("which cook", { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

/**
 * Spawn cook process for a mission.
 *
 * Phase determines cook invocation:
 * - PLANNING: --max-iterations 1 (generate plan only)
 * - RUNNING: full research loop (default iterations)
 * - CHECKPOINT_WAITING → RUNNING: --max-iterations 1 (one more cycle)
 */
export async function spawnCook(
  missionId: string,
  phase: "planning" | "running" | "checkpoint-resume"
): Promise<void> {
  if (activeProcess) {
    throw new Error(
      `Another mission is already running: ${activeMissionId}`
    );
  }

  const missionPath = getMissionPath(missionId);
  const kbPath = getMissionKBPath(missionId);

  if (!existsSync(missionPath)) {
    throw new Error(`Mission directory not found: ${missionPath}`);
  }

  const state = await readMissionState(missionId);
  if (!state) throw new Error(`Mission ${missionId} not found`);

  // Determine cook arguments based on phase
  const workPrompt = "Continue research";
  const reviewPrompt =
    "Review current status and verify if we achieved the target mission";
  const gatePrompt =
    "DONE if we achieved the target mission, else ITERATE";

  const args = [workPrompt, "review", reviewPrompt, gatePrompt];

  if (phase === "planning" || phase === "checkpoint-resume") {
    args.push("--max-iterations", "1");
  }

  // Log file for audit trail
  const logPath = join(missionPath, "agent.log");

  // Spawn cook in the mission's KB directory
  const proc = spawn("cook", args, {
    cwd: missionPath,
    detached: true, // Create process group for clean kill
    stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env },
  });

  if (!proc.pid) {
    throw new Error("Failed to spawn cook process");
  }

  activeProcess = proc;
  activeMissionId = missionId;
  lastArtifactTime = Date.now();

  // Capture stdout/stderr to log file (concurrent appends are safe for append-only files)
  proc.stdout?.on("data", (data: Buffer) => {
    appendFile(logPath, `[stdout ${new Date().toISOString()}] ${data}`).catch(() => {});
  });
  proc.stderr?.on("data", (data: Buffer) => {
    appendFile(logPath, `[stderr ${new Date().toISOString()}] ${data}`).catch(() => {});
  });

  // Transition mission state
  const newPhase: MissionPhase =
    phase === "planning" ? "PLANNING" : "RUNNING";
  await transitionMission(missionId, newPhase, {
    pid: proc.pid,
    pgid: proc.pid, // Process group ID = PID for detached processes
  });

  // Start stall detection
  startStallDetection(missionId);

  // Handle process exit
  proc.on("exit", async (code, signal) => {
    activeProcess = null;
    activeMissionId = null;
    stopStallDetection();

    const currentState = await readMissionState(missionId);
    if (!currentState) return;

    // Don't transition if already KILLED
    if (currentState.phase === "KILLED") return;

    if (code === 0) {
      if (phase === "planning") {
        // Planning complete — check if BACKLOG.md was populated
        await transitionMission(missionId, "PLAN_READY");
      } else if (
        currentState.phase === "RUNNING" &&
        state.autonomyLevel === "checkpoint"
      ) {
        // Checkpoint mode — cook finished one iteration
        await transitionMission(missionId, "CHECKPOINT_WAITING");
        if (currentState.slackWebhook) {
          await notifyCheckpoint(currentState);
        }
      } else {
        // Full run complete
        await transitionMission(missionId, "COMPLETED", {
          completedAt: new Date().toISOString(),
        });
        if (currentState.slackWebhook) {
          await notifyCompleted(currentState);
        }
      }
    } else {
      // Process failed
      const reason = signal
        ? `Killed by signal ${signal}`
        : `Exited with code ${code}`;
      await transitionMission(missionId, "FAILED_RECOVERABLE", {
        failureReason: reason,
      });
      if (currentState.slackWebhook) {
        await notifyFailed(currentState, reason);
      }
    }
  });

  proc.on("error", async (err) => {
    activeProcess = null;
    activeMissionId = null;
    stopStallDetection();

    const reason =
      err.message === "spawn cook ENOENT"
        ? "cook CLI not found. Install with: npm install -g @let-it-cook/cli"
        : err.message;

    await transitionMission(missionId, "FAILED_RECOVERABLE", {
      failureReason: reason,
    });

    const currentState = await readMissionState(missionId);
    if (currentState?.slackWebhook) {
      await notifyFailed(currentState, reason);
    }
  });
}

/**
 * Kill the active mission's cook process group.
 * Uses process.kill(-pgid) to kill the entire process tree.
 */
export async function killMission(missionId: string): Promise<void> {
  const state = await readMissionState(missionId);
  if (!state) throw new Error(`Mission ${missionId} not found`);

  if (activeMissionId !== missionId || !activeProcess) {
    // Process might have already exited — just update state
    if (
      state.phase !== "COMPLETED" &&
      state.phase !== "KILLED" &&
      state.phase !== "FAILED_RECOVERABLE"
    ) {
      await transitionMission(missionId, "KILLED");
    }
    return;
  }

  stopStallDetection();

  // Kill entire process group
  try {
    if (state.pgid) {
      process.kill(-state.pgid, "SIGTERM");
    } else if (activeProcess.pid) {
      process.kill(-activeProcess.pid, "SIGTERM");
    }
  } catch {
    // Process already exited
  }

  await transitionMission(missionId, "KILLED");
  activeProcess = null;
  activeMissionId = null;
}

/**
 * Start stall detection — checks every 5 minutes if new artifacts appeared.
 */
function startStallDetection(missionId: string): void {
  stopStallDetection();

  const STALL_THRESHOLD_MS = 30 * 60 * 1000; // 30 minutes
  const CHECK_INTERVAL_MS = 5 * 60 * 1000; // 5 minutes

  stallTimer = setInterval(async () => {
    if (activeMissionId !== missionId) {
      stopStallDetection();
      return;
    }

    const state = await readMissionState(missionId);
    if (!state || state.phase !== "RUNNING") return;

    // Check if cook process is still alive
    if (activeProcess?.pid) {
      try {
        process.kill(activeProcess.pid, 0); // Signal 0 = check existence
      } catch {
        // Process is dead but we didn't get the exit event yet
        return;
      }
    }

    // Check for new KB artifacts
    const kbPath = getMissionKBPath(missionId);
    const kbState = await readKBState(kbPath);
    const totalArtifacts =
      kbState.artifactCounts.hypotheses +
      kbState.artifactCounts.experiments +
      kbState.artifactCounts.findings;

    const now = Date.now();
    if (now - lastArtifactTime > STALL_THRESHOLD_MS && totalArtifacts > 0) {
      await transitionMission(missionId, "STALLED");
      if (state.slackWebhook) {
        await notifyStalled(state);
      }
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
 * Check if the active process is still running.
 */
export function isProcessAlive(): boolean {
  if (!activeProcess?.pid) return false;
  try {
    process.kill(activeProcess.pid, 0);
    return true;
  } catch {
    return false;
  }
}

/**
 * On server startup, check for missions that were running when the server stopped.
 * Mark them as FAILED_RECOVERABLE since the cook process is gone.
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
      // Check if the PID is still alive
      if (mission.pid) {
        try {
          process.kill(mission.pid, 0);
          // Process is still running — shouldn't happen after restart
          // but leave it alone
          continue;
        } catch {
          // Process is dead — mark as recoverable
        }
      }

      await transitionMission(mission.id, "FAILED_RECOVERABLE", {
        failureReason: "Server restarted while mission was running",
      });
    }
  }
}
