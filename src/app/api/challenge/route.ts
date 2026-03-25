import { NextResponse } from "next/server";

function getMissionDir(): string {
  return process.env.MISSION_DIR || process.cwd();
}

export async function POST() {
  const cwd = getMissionDir();

  try {
    const { query } = await import("@anthropic-ai/claude-agent-sdk");

    // Fire-and-forget: run challenge review in background
    const challengePrompt =
      "Run a challenge review of the current research direction. " +
      "Spawn the devil-advocate agent via /challenge with target 'Research direction'. " +
      "Write the review to kb/reports/CR{next}-challenge-review.md.";

    // Don't await — this runs detached
    (async () => {
      try {
        for await (const _msg of query({
          prompt: challengePrompt,
          options: {
            cwd,
            model: process.env.LIMINA_MODEL || "claude-opus-4-6",
            thinking: { type: "adaptive" },
            permissionMode: "bypassPermissions" as const,
            allowDangerouslySkipPermissions: true,
            tools: { type: "preset", preset: "claude_code" },
            settingSources: ["project" as const],
            maxTurns: 10,
            env: {
              ...process.env,
              CLAUDE_AGENT_SDK_CLIENT_APP: "limina/1.0",
            },
          } as any,
        })) {
          // Consume stream
        }
      } catch {
        // Non-fatal: challenge review failure doesn't affect the main mission
      }
    })();

    return NextResponse.json({ success: true, message: "Challenge review triggered" });
  } catch (err) {
    return NextResponse.json(
      { error: err instanceof Error ? err.message : "Failed to trigger challenge" },
      { status: 500 }
    );
  }
}
