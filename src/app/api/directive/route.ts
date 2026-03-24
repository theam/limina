import { NextRequest, NextResponse } from "next/server";
import { readFile, writeFile, rename } from "fs/promises";
import { join } from "path";
import { parseDirectives } from "../../../lib/kb-parser";
import { getMissionRunner } from "../../../lib/mission-runner";

function getMissionDir(): string {
  return process.env.MISSION_DIR || process.cwd();
}

export async function POST(request: NextRequest) {
  const body = await request.json();
  const { instruction, priority = "NORMAL" } = body;

  if (!instruction || typeof instruction !== "string" || !instruction.trim()) {
    return NextResponse.json(
      { error: "instruction is required" },
      { status: 400 }
    );
  }

  const validPriorities = ["HIGH", "NORMAL", "LOW"];
  if (!validPriorities.includes(priority)) {
    return NextResponse.json(
      { error: "priority must be HIGH, NORMAL, or LOW" },
      { status: 400 }
    );
  }

  const directivesPath = join(getMissionDir(), "kb/mission/DIRECTIVES.md");

  try {
    // Read existing file or create with header
    let content = "";
    try {
      content = await readFile(directivesPath, "utf-8");
    } catch {
      content =
        "# CEO Directives\n\n_Strategic instructions from the CEO to incorporate into ongoing work._\n";
    }

    // Determine next DIR-NNN ID
    const existing = parseDirectives(content);
    const maxNum = existing.reduce((max, d) => {
      const num = parseInt(d.id.replace("DIR-", ""), 10);
      return num > max ? num : max;
    }, 0);
    const nextId = `DIR-${String(maxNum + 1).padStart(3, "0")}`;

    // Build title from first line of instruction (truncated)
    const titleLine = instruction.trim().split("\n")[0];
    const title =
      titleLine.length > 60 ? titleLine.slice(0, 57) + "..." : titleLine;

    // Append new directive
    const entry = `\n## ${nextId}: ${title}\n> **Priority**: ${priority}\n> **Submitted**: ${new Date().toISOString()}\n> **Status**: PENDING\n\n${instruction.trim()}\n`;
    const updated = content + entry;

    // Atomic write
    const tmp = `${directivesPath}.tmp.${Date.now()}`;
    await writeFile(tmp, updated);
    await rename(tmp, directivesPath);

    // Inject into running agent session if active
    try {
      const runner = getMissionRunner();
      if (runner) {
        await runner.injectDirective(instruction.trim());
      }
    } catch {
      // Runner not available — directive will be picked up from file at next iteration
    }

    return NextResponse.json({ success: true, id: nextId });
  } catch (err) {
    return NextResponse.json(
      {
        error:
          err instanceof Error ? err.message : "Failed to save directive",
      },
      { status: 500 }
    );
  }
}
