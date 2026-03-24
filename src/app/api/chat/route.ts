import { NextRequest } from "next/server";

export const dynamic = "force-dynamic";

const SIDECAR_SYSTEM_PROMPT = `You are Limina's research assistant. You share the same mission directory as the autonomous research agent that is currently running.

Your job:
- Answer questions about the research mission's progress, findings, and decisions
- Read files in kb/ to give accurate, grounded answers
- If the user wants to change the agent's direction, priorities, or approach, send a directive by calling the Bash tool with: curl -s -X POST http://localhost:${process.env.PORT || 3000}/api/directive -H "Content-Type: application/json" -d '{"instruction":"<the directive text>","priority":"NORMAL"}'
- Be concise and direct — the user is busy monitoring research

You can see:
- kb/mission/CHALLENGE.md — the research objective
- kb/mission/BACKLOG.md — current tasks and priorities
- kb/mission/DIRECTIVES.md — CEO directives
- kb/research/ — hypotheses, experiments, findings
- kb/reports/ — challenge reviews, strategic reviews
- agent.log — raw agent activity log

Start by reading kb/INDEX.md and kb/mission/BACKLOG.md to understand the current state before answering.`;

export async function POST(request: NextRequest) {
  const body = await request.json();
  const { message, sessionId } = body;

  if (!message || typeof message !== "string") {
    return new Response(JSON.stringify({ error: "message is required" }), {
      status: 400,
      headers: { "Content-Type": "application/json" },
    });
  }

  const missionDir = process.env.MISSION_DIR || process.cwd();

  const encoder = new TextEncoder();
  const stream = new ReadableStream({
    async start(controller) {
      try {
        const { query } = await import("@anthropic-ai/claude-agent-sdk");

        const prompt = sessionId
          ? message
          : `${SIDECAR_SYSTEM_PROMPT}\n\nUser: ${message}`;

        const options: Record<string, unknown> = {
          cwd: missionDir,
          model: process.env.LIMINA_MODEL || "claude-opus-4-6",
          thinking: { type: "adaptive" },
          permissionMode: "bypassPermissions",
          allowDangerouslySkipPermissions: true,
          tools: { type: "preset", preset: "claude_code" },
          settingSources: ["project"],
          env: {
            ...process.env,
            CLAUDE_AGENT_SDK_CLIENT_APP: "limina-talk/1.0",
          },
        };

        if (sessionId) {
          options.resume = sessionId;
        }

        let capturedSessionId = sessionId || "";

        for await (const msg of query({
          prompt,
          options: options as any,
        })) {
          // Capture session ID from init
          if (
            msg.type === "system" &&
            "subtype" in msg &&
            msg.subtype === "init"
          ) {
            capturedSessionId = (msg as any).session_id;
            controller.enqueue(
              encoder.encode(
                JSON.stringify({
                  type: "session",
                  sessionId: capturedSessionId,
                }) + "\n"
              )
            );
          }

          // Stream assistant text
          if (msg.type === "assistant" && "message" in msg) {
            const content = (msg as any).message?.content;
            if (Array.isArray(content)) {
              for (const block of content) {
                if (block.type === "text" && block.text) {
                  controller.enqueue(
                    encoder.encode(
                      JSON.stringify({ type: "text", text: block.text }) + "\n"
                    )
                  );
                }
              }
            }
          }

          // Signal completion
          if (msg.type === "result") {
            controller.enqueue(
              encoder.encode(
                JSON.stringify({
                  type: "done",
                  sessionId: capturedSessionId,
                }) + "\n"
              )
            );
          }
        }

        controller.close();
      } catch (err) {
        controller.enqueue(
          encoder.encode(
            JSON.stringify({
              type: "error",
              error:
                err instanceof Error ? err.message : "Chat query failed",
            }) + "\n"
          )
        );
        controller.close();
      }
    },
  });

  return new Response(stream, {
    headers: {
      "Content-Type": "application/x-ndjson",
      "Cache-Control": "no-cache",
      Connection: "keep-alive",
    },
  });
}
