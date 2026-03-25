import { existsSync } from "fs";
import { join } from "path";
import {
  getMissionPath,
  transitionMission,
} from "./mission";

// ─── Agent service URL (set by CLI via env) ───

function getServiceUrl(): string {
  const url = process.env.AGENT_SERVICE_URL;
  if (!url) throw new Error("AGENT_SERVICE_URL is not set");
  return url;
}

/**
 * Start a mission via the agent service.
 */
export async function startMission(
  missionId: string,
  phase: "planning" | "running" | "checkpoint-resume",
  missionDir?: string
): Promise<void> {
  const missionPath = missionDir || getMissionPath(missionId);
  if (!existsSync(missionPath)) {
    throw new Error(`Mission directory not found: ${missionPath}`);
  }

  const res = await fetch(`${getServiceUrl()}/start`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ missionId, phase, missionDir: missionPath }),
  });

  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    throw new Error(body.error || `Agent service returned ${res.status}`);
  }
}

/**
 * Kill the active mission via the agent service.
 */
export async function killMission(missionId: string): Promise<void> {
  try {
    await fetch(`${getServiceUrl()}/kill`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
    });
  } catch {
    // Agent service may be unreachable — try SIGTERM via PID
    try {
      const { readFile } = await import("fs/promises");
      const missionPath = getMissionPath(missionId);
      const raw = await readFile(join(missionPath, "mission.json"), "utf-8");
      const state = JSON.parse(raw);
      if (state.pid) {
        process.kill(state.pid, "SIGTERM");
      }
    } catch {}
  }
}

/**
 * Send a directive to the running agent via the agent service.
 */
export function sendDirective(text: string): void {
  // Fire-and-forget — don't block the API route
  fetch(`${getServiceUrl()}/directive`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ text }),
  }).catch(() => {
    // Agent service unreachable — directive is already in DIRECTIVES.md
  });
}

/**
 * Get the currently active mission ID from the agent service.
 */
export async function getActiveMissionId(): Promise<string | null> {
  try {
    const res = await fetch(`${getServiceUrl()}/health`);
    if (!res.ok) return null;
    const data = await res.json();
    return data.missionId || null;
  } catch {
    return null;
  }
}

/**
 * Check if the agent service has an active running mission.
 */
export async function isProcessAlive(): Promise<boolean> {
  try {
    const res = await fetch(`${getServiceUrl()}/health`);
    if (!res.ok) return false;
    const data = await res.json();
    return data.running === true;
  } catch {
    return false;
  }
}

/**
 * On server startup, check for missions that were running when the server stopped.
 * Kill orphan agent service processes and transition stale missions.
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
      // Kill orphan agent service if PID is recorded
      if (mission.pid) {
        try {
          process.kill(mission.pid, "SIGTERM");
        } catch {
          // Already dead
        }
      }

      await transitionMission(mission.id, "FAILED_RECOVERABLE", {
        failureReason:
          "Server restarted while mission was running. Session can be resumed.",
      });
    }
  }
}

/**
 * Check if the agent service is reachable.
 */
export async function checkAgentSdkAvailable(): Promise<boolean> {
  try {
    const res = await fetch(`${getServiceUrl()}/health`);
    return res.ok;
  } catch {
    return false;
  }
}

// updateLastArtifactTime is now handled by the agent service directly
export function updateLastArtifactTime(): void {
  // No-op in UI server — stall detection runs in agent service
}
