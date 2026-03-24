import { NextRequest, NextResponse } from "next/server";
import { readFile, writeFile, rename } from "fs/promises";
import { join } from "path";

export const dynamic = "force-dynamic";

function getMissionDir(): string {
  return process.env.MISSION_DIR || process.cwd();
}

export async function POST(request: NextRequest) {
  let body: { requestId: string; response: string };
  try {
    body = await request.json();
  } catch {
    return NextResponse.json({ error: "Invalid JSON body" }, { status: 400 });
  }

  if (!body.requestId || !body.response) {
    return NextResponse.json(
      { error: "requestId and response are required" },
      { status: 400 }
    );
  }

  // Validate requestId format to prevent regex injection
  if (!/^REQ-\d+$/.test(body.requestId)) {
    return NextResponse.json(
      { error: `Invalid request ID format: ${body.requestId}` },
      { status: 400 }
    );
  }

  const reqPath = join(getMissionDir(), "kb/mission/CEO_REQUESTS.md");

  try {
    const content = await readFile(reqPath, "utf-8");

    // Find the request section and update its status
    const escapedResponse = body.response.replace(/\$/g, "$$$$");
    const updated = content.replace(
      new RegExp(
        `(## ${body.requestId}:.*?\\n>\\s+\\*\\*Status\\*\\*:\\s*)PENDING`,
        "s"
      ),
      `$1RESOLVED\n> **Response**: ${escapedResponse}\n> **Resolved at**: ${new Date().toISOString()}`
    );

    if (updated === content) {
      return NextResponse.json(
        { error: `CEO request ${body.requestId} not found or already resolved` },
        { status: 409 }
      );
    }

    // Atomic write
    const tmpPath = `${reqPath}.tmp.${Date.now()}`;
    await writeFile(tmpPath, updated);
    await rename(tmpPath, reqPath);

    return NextResponse.json({ success: true });
  } catch (err) {
    const message =
      err instanceof Error ? err.message : "Failed to respond to escalation";
    return NextResponse.json({ error: message }, { status: 500 });
  }
}
