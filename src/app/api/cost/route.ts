import { NextRequest, NextResponse } from "next/server";
import { readFile, writeFile, rename } from "fs/promises";
import { join } from "path";

export const dynamic = "force-dynamic";

function getMissionDir(): string {
  return process.env.MISSION_DIR || process.cwd();
}

interface CostState {
  budget: number | null;
  spent: number;
  phases: Array<{ name: string; startedAt: string; endedAt?: string }>;
}

async function readCostFile(missionDir: string): Promise<CostState> {
  const costPath = join(missionDir, "cost.json");
  try {
    const content = await readFile(costPath, "utf-8");
    return JSON.parse(content) as CostState;
  } catch {
    // Fall back to extracting budget from mission.json
    try {
      const missionContent = await readFile(
        join(missionDir, "mission.json"),
        "utf-8"
      );
      const mission = JSON.parse(missionContent);
      const budgetStr = mission.budget || mission.estimatedCost || "";
      // Try to parse numeric budget from string like "$100" or "100"
      const match = budgetStr.match?.(/([\d.]+)/);
      const budget = match ? parseFloat(match[1]) : null;
      return { budget, spent: 0, phases: [] };
    } catch {
      return { budget: null, spent: 0, phases: [] };
    }
  }
}

async function writeCostFile(
  missionDir: string,
  state: CostState
): Promise<void> {
  const costPath = join(missionDir, "cost.json");
  const tmpPath = `${costPath}.tmp.${Date.now()}`;
  await writeFile(tmpPath, JSON.stringify(state, null, 2));
  await rename(tmpPath, costPath);
}

export async function GET() {
  const missionDir = getMissionDir();
  const costState = await readCostFile(missionDir);
  return NextResponse.json(costState);
}

export async function POST(request: NextRequest) {
  let body: { budget: number };
  try {
    body = await request.json();
  } catch {
    return NextResponse.json({ error: "Invalid JSON body" }, { status: 400 });
  }

  if (typeof body.budget !== "number" || body.budget < 0) {
    return NextResponse.json(
      { error: "budget must be a non-negative number" },
      { status: 400 }
    );
  }

  const missionDir = getMissionDir();
  const costState = await readCostFile(missionDir);
  costState.budget = body.budget;
  await writeCostFile(missionDir, costState);

  return NextResponse.json({ success: true, budget: body.budget });
}
