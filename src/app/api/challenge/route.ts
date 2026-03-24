import { NextResponse } from "next/server";
import { spawn } from "child_process";

function getMissionDir(): string {
  return process.env.MISSION_DIR || process.cwd();
}

export async function POST() {
  const cwd = getMissionDir();

  try {
    const proc = spawn(
      "cook",
      [
        "Run /challenge with target 'Research direction'",
        "review",
        "Read the CR report and assess whether critical issues were addressed",
        "DONE if no critical issues remain, else ITERATE",
        "--max-iterations", "1",
        "--agent", "claude",
      ],
      {
        cwd,
        detached: true,
        stdio: "ignore",
      }
    );
    proc.unref();

    return NextResponse.json({ success: true, message: "Challenge review triggered" });
  } catch (err) {
    return NextResponse.json(
      { error: err instanceof Error ? err.message : "Failed to trigger challenge" },
      { status: 500 }
    );
  }
}
