import { NextRequest, NextResponse } from "next/server";
import { readKBState } from "@/lib/kb-parser";
import { join } from "path";

export const dynamic = "force-dynamic";

export async function GET(
  _request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const { id } = await params;
  const missionDir = process.env.MISSION_DIR || process.cwd();
  const kbPath = join(missionDir, "kb");

  try {
    const kbState = await readKBState(kbPath);
    const artifact = kbState.artifacts.find((a) => a.id === id);

    if (!artifact) {
      return NextResponse.json({ error: "Artifact not found" }, { status: 404 });
    }

    return NextResponse.json({ artifact });
  } catch {
    return NextResponse.json({ error: "Failed to read KB" }, { status: 500 });
  }
}
