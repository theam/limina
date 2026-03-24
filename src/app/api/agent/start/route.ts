import { NextRequest, NextResponse } from "next/server";
import { readFile } from "fs/promises";
import { join } from "path";
import { startMission, getActiveMissionId } from "../../../../lib/mission-runner";

function getMissionDir(): string {
  return process.env.MISSION_DIR || process.cwd();
}

export async function POST(request: NextRequest) {
  const body = await request.json().catch(() => ({}));
  const phase = body.phase || "running";

  if (!["planning", "running", "checkpoint-resume"].includes(phase)) {
    return NextResponse.json(
      { error: "phase must be planning, running, or checkpoint-resume" },
      { status: 400 }
    );
  }

  // Check if already running
  const activeId = getActiveMissionId();
  if (activeId) {
    return NextResponse.json(
      { error: `Mission ${activeId} is already running` },
      { status: 409 }
    );
  }

  // Read mission ID from mission.json
  const missionDir = getMissionDir();
  try {
    const raw = await readFile(join(missionDir, "mission.json"), "utf-8");
    const state = JSON.parse(raw);
    const missionId = state.id;

    if (!missionId) {
      return NextResponse.json(
        { error: "No mission ID found in mission.json" },
        { status: 400 }
      );
    }

    await startMission(missionId, phase, missionDir);

    return NextResponse.json({ success: true, missionId });
  } catch (err) {
    return NextResponse.json(
      {
        error:
          err instanceof Error ? err.message : "Failed to start agent",
      },
      { status: 500 }
    );
  }
}
