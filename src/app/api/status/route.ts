import { NextResponse } from "next/server";
import { readFile } from "fs/promises";
import { join } from "path";
import { readKBState } from "@/lib/kb-parser";

export const dynamic = "force-dynamic";

export async function GET() {
  const missionDir = process.env.MISSION_DIR || process.cwd();
  const missionJsonPath = join(missionDir, "mission.json");
  const kbPath = join(missionDir, "kb");

  // Read mission.json
  let mission = null;
  try {
    const raw = await readFile(missionJsonPath, "utf-8");
    mission = JSON.parse(raw);
  } catch {
    // mission.json doesn't exist or is malformed
  }

  // Read KB state
  let kb = null;
  try {
    kb = await readKBState(kbPath);
  } catch {
    // KB directory doesn't exist yet
  }

  // Extract pending escalations from CEO requests
  const pendingEscalations =
    kb?.ceoRequests.filter((r) => r.status === "PENDING") ?? [];

  return NextResponse.json({
    mission,
    kb,
    pendingEscalations,
  });
}
