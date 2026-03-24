import { NextRequest, NextResponse } from "next/server";
import { readFile, writeFile, rename } from "fs/promises";
import { join } from "path";

function getMissionDir(): string {
  return process.env.MISSION_DIR || process.cwd();
}

export async function POST(request: NextRequest) {
  const body = await request.json();
  const { artifactId, comment } = body;

  if (!artifactId || !comment) {
    return NextResponse.json(
      { error: "artifactId and comment are required" },
      { status: 400 }
    );
  }

  const feedbackPath = join(getMissionDir(), "kb/mission/FEEDBACK.md");

  try {
    let content = "";
    try {
      content = await readFile(feedbackPath, "utf-8");
    } catch {
      content = "# Feedback\n\n";
    }

    const entry = `\n## ${new Date().toISOString()} — ${artifactId}\n\n${comment}\n`;
    const updated = content + entry;

    const tmp = `${feedbackPath}.tmp.${Date.now()}`;
    await writeFile(tmp, updated);
    await rename(tmp, feedbackPath);

    return NextResponse.json({ success: true });
  } catch (err) {
    return NextResponse.json(
      { error: err instanceof Error ? err.message : "Failed to save feedback" },
      { status: 500 }
    );
  }
}
