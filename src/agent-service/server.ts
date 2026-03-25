/**
 * Agent Service — standalone Fastify server that runs the MissionRunner.
 *
 * Runs in a dedicated process, separate from the Next.js UI server.
 * The UI server proxies control commands here via HTTP.
 *
 * Env vars:
 *   AGENT_PORT   — port to listen on (required)
 *   MISSION_DIR  — mission directory (required)
 */

import Fastify from "fastify";
import { readFile, writeFile, rename } from "fs/promises";
import { join } from "path";
import { MissionRunner } from "../lib/mission-runner-core";
import { readKBState } from "../lib/kb-parser";
import { notifyStalled } from "../lib/notify";
import type { MissionState } from "../lib/mission";

const port = parseInt(process.env.AGENT_PORT || "0", 10);
const missionDir = process.env.MISSION_DIR || process.cwd();

if (!process.env.AGENT_PORT) {
  console.error("[agent-service] AGENT_PORT is required");
  process.exit(1);
}

// ─── State ───

let runner: MissionRunner | null = null;
let activeMissionId: string | null = null;
let stallTimer: ReturnType<typeof setInterval> | null = null;
let lastArtifactTime: number = Date.now();

// ─── Stall detection ───

function startStallDetection(missionId: string, missionPath: string): void {
  stopStallDetection();

  const STALL_THRESHOLD_MS = 30 * 60 * 1000; // 30 minutes
  const CHECK_INTERVAL_MS = 5 * 60 * 1000; // 5 minutes

  stallTimer = setInterval(async () => {
    if (activeMissionId !== missionId) {
      stopStallDetection();
      return;
    }

    const kbPath = join(missionPath, "kb");
    try {
      const kbState = await readKBState(kbPath);
      const totalArtifacts =
        kbState.artifactCounts.hypotheses +
        kbState.artifactCounts.experiments +
        kbState.artifactCounts.findings;

      const now = Date.now();
      if (now - lastArtifactTime > STALL_THRESHOLD_MS && totalArtifacts > 0) {
        try {
          const raw = await readFile(join(missionPath, "mission.json"), "utf-8");
          const state = JSON.parse(raw) as MissionState;
          if (state.slackWebhook) {
            await notifyStalled(state);
          }
        } catch {}
      }
    } catch {
      // KB not readable yet
    }
  }, CHECK_INTERVAL_MS);
}

function stopStallDetection(): void {
  if (stallTimer) {
    clearInterval(stallTimer);
    stallTimer = null;
  }
}

// ─── Fastify server ───

const app = Fastify({ logger: false });

app.post<{
  Body: { missionId: string; phase: string; missionDir: string };
}>("/start", async (request, reply) => {
  const { missionId, phase, missionDir: dir } = request.body;

  if (runner?.isRunning()) {
    return reply.status(409).send({ error: `Mission ${activeMissionId} is already running` });
  }

  const validPhases = ["planning", "running", "checkpoint-resume"] as const;
  if (!validPhases.includes(phase as any)) {
    return reply.status(400).send({ error: "Invalid phase" });
  }

  runner = new MissionRunner(missionId, dir);
  activeMissionId = missionId;
  lastArtifactTime = Date.now();

  // Persist PID for orphan cleanup
  try {
    const statePath = join(dir, "mission.json");
    const raw = await readFile(statePath, "utf-8");
    const state = JSON.parse(raw) as MissionState;
    state.pid = process.pid;
    const tmp = `${statePath}.tmp.${Date.now()}`;
    await writeFile(tmp, JSON.stringify(state, null, 2));
    await rename(tmp, statePath);
  } catch {}

  startStallDetection(missionId, dir);

  // Run in background — don't await
  runner.run(phase as any).finally(() => {
    if (activeMissionId === missionId) {
      runner = null;
      activeMissionId = null;
      stopStallDetection();
    }
  });

  return { success: true, missionId };
});

app.post<{
  Body: { text: string };
}>("/directive", async (request, reply) => {
  const { text } = request.body;

  if (!runner?.isRunning()) {
    return reply.status(404).send({ error: "No active mission" });
  }

  runner.injectDirective(text);
  return { success: true };
});

app.post("/kill", async (_request, reply) => {
  if (!runner) {
    return reply.status(404).send({ error: "No active mission" });
  }

  runner.kill();
  stopStallDetection();

  const id = activeMissionId;
  runner = null;
  activeMissionId = null;

  return { success: true, missionId: id };
});

app.get("/health", async () => {
  return {
    alive: true,
    missionId: activeMissionId,
    running: runner?.isRunning() ?? false,
    pid: process.pid,
  };
});

// ─── Chat (sidecar assistant) ───

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

app.post<{
  Body: { message: string; sessionId?: string };
}>("/chat", async (request, reply) => {
  const { message, sessionId } = request.body;

  if (!message || typeof message !== "string") {
    return reply.status(400).send({ error: "message is required" });
  }

  // Lazy import SDK
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

  // Stream NDJSON back
  reply.raw.writeHead(200, {
    "Content-Type": "application/x-ndjson",
    "Cache-Control": "no-cache",
    Connection: "keep-alive",
  });

  let capturedSessionId = sessionId || "";

  try {
    for await (const msg of query({ prompt, options: options as any })) {
      if (
        msg.type === "system" &&
        "subtype" in msg &&
        msg.subtype === "init"
      ) {
        capturedSessionId = (msg as any).session_id;
        reply.raw.write(
          JSON.stringify({ type: "session", sessionId: capturedSessionId }) + "\n"
        );
      }

      if (msg.type === "assistant" && "message" in msg) {
        const content = (msg as any).message?.content;
        if (Array.isArray(content)) {
          for (const block of content) {
            if (block.type === "text" && block.text) {
              reply.raw.write(
                JSON.stringify({ type: "text", text: block.text }) + "\n"
              );
            }
          }
        }
      }

      if (msg.type === "result") {
        reply.raw.write(
          JSON.stringify({ type: "done", sessionId: capturedSessionId }) + "\n"
        );
      }
    }
  } catch (err) {
    reply.raw.write(
      JSON.stringify({
        type: "error",
        error: err instanceof Error ? err.message : "Chat query failed",
      }) + "\n"
    );
  }

  reply.raw.end();
  return reply;
});

// ─── Challenge review ───

app.post("/challenge", async (_request, reply) => {
  // Lazy import SDK
  const { query } = await import("@anthropic-ai/claude-agent-sdk");

  const challengePrompt =
    "Run a challenge review of the current research direction. " +
    "Spawn the devil-advocate agent via /challenge with target 'Research direction'. " +
    "Write the review to kb/reports/CR{next}-challenge-review.md.";

  // Fire-and-forget — don't await
  (async () => {
    try {
      for await (const _msg of query({
        prompt: challengePrompt,
        options: {
          cwd: missionDir,
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
      // Non-fatal
    }
  })();

  return { success: true, message: "Challenge review triggered" };
});

// ─── Start ───

async function main() {
  await app.listen({ port, host: "127.0.0.1" });
  console.log(`[agent-service] Listening on http://127.0.0.1:${port}`);
}

main().catch((err) => {
  console.error("[agent-service] Failed to start:", err);
  process.exit(1);
});
