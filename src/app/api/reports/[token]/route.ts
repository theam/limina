import { NextRequest, NextResponse } from "next/server";
import { findMissionByShareToken, getMissionKBPath } from "@/lib/mission";
import { readKBState } from "@/lib/kb-parser";
import { readFile } from "fs/promises";
import { join } from "path";

// Public endpoint — no auth required (read-only via share token)
export async function GET(
  _request: NextRequest,
  { params }: { params: Promise<{ token: string }> }
) {
  const { token } = await params;

  const mission = await findMissionByShareToken(token);
  if (!mission) {
    return NextResponse.json({ error: "Report not found" }, { status: 404 });
  }

  if (mission.phase !== "COMPLETED") {
    return NextResponse.json(
      { error: "Report not yet available" },
      { status: 404 }
    );
  }

  const kbPath = getMissionKBPath(mission.id);
  const kbState = await readKBState(kbPath);

  let challenge = "";
  try {
    challenge = await readFile(join(kbPath, "mission/CHALLENGE.md"), "utf-8");
  } catch {
    // Missing
  }

  const findings = kbState.artifacts
    .filter((a) => a.type === "finding")
    .sort((a, b) => {
      const impactOrder: Record<string, number> = { HIGH: 3, MEDIUM: 2, LOW: 1 };
      return (
        (impactOrder[b.metadata.impact?.toUpperCase() || ""] || 0) -
        (impactOrder[a.metadata.impact?.toUpperCase() || ""] || 0)
      );
    });

  const hypotheses = kbState.artifacts.filter((a) => a.type === "hypothesis");
  const experiments = kbState.artifacts.filter((a) => a.type === "experiment");

  let durationHours: string | null = null;
  if (mission.createdAt && mission.completedAt) {
    const hours =
      (new Date(mission.completedAt).getTime() -
        new Date(mission.createdAt).getTime()) /
      (1000 * 60 * 60);
    durationHours = `${hours.toFixed(1)}h`;
  }

  return NextResponse.json({
    objective: mission.objective,
    challenge,
    findings: findings.map((f) => ({
      id: f.id,
      title: f.title,
      impact: f.metadata.impact || "unknown",
      content: f.content,
    })),
    summary: {
      duration: durationHours,
      hypothesesTested: hypotheses.length,
      experimentsRun: experiments.length,
      findingsProduced: findings.length,
    },
    completedAt: mission.completedAt,
  });
}
