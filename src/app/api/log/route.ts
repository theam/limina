import { NextRequest, NextResponse } from "next/server";
import { open, stat } from "fs/promises";
import { join } from "path";

export const dynamic = "force-dynamic";

interface LogEntry {
  timestamp: string;
  type: string;
  summary: string;
}

const NOISE_TYPES = new Set([
  "partial_assistant",
  "hook_started",
  "hook_progress",
  "hook_response",
  "rate_limit",
  "keepalive",
  "files_persisted",
  "prompt_suggestion",
  "elicitation_complete",
]);

function summarizeMessage(msg: Record<string, unknown>): LogEntry | null {
  const type = msg.type as string;

  // Skip noisy internal types
  if (NOISE_TYPES.has(type)) return null;

  // System init
  if (type === "system" && (msg as any).subtype === "init") {
    const model = (msg as any).model || "unknown";
    return { timestamp: "", type: "system", summary: `Session started — ${model}` };
  }

  // System status (compacting etc.)
  if (type === "system" && (msg as any).subtype === "status") {
    const status = (msg as any).status;
    if (status === "compacting") {
      return { timestamp: "", type: "system", summary: "Compacting context" };
    }
    return null;
  }

  // Skip other system subtypes
  if (type === "system") return null;

  // Assistant message
  if (type === "assistant") {
    const content = (msg as any).message?.content;
    if (Array.isArray(content)) {
      // Extract text blocks
      const textParts: string[] = [];
      for (const block of content) {
        if (block.type === "text" && block.text) {
          textParts.push(block.text);
        } else if (block.type === "tool_use") {
          const name = block.name || "unknown";
          const input = block.input || {};
          const detail = summarizeToolInput(name, input);
          return { timestamp: "", type: "tool", summary: `${name}${detail ? ` — ${detail}` : ""}` };
        }
      }
      const text = textParts.join(" ").trim();
      if (!text) return null;
      const truncated = text.length > 200 ? text.slice(0, 197) + "..." : text;
      return { timestamp: "", type: "assistant", summary: truncated };
    }
    return null;
  }

  // User message
  if (type === "user") {
    const content = (msg as any).message?.content;
    let text = "";
    if (typeof content === "string") {
      text = content;
    } else if (Array.isArray(content)) {
      text = content
        .filter((b: any) => b.type === "text")
        .map((b: any) => b.text)
        .join(" ");
    }
    if (text.includes("CEO DIRECTIVE")) {
      return { timestamp: "", type: "directive", summary: text.slice(0, 200) };
    }
    const truncated = text.length > 200 ? text.slice(0, 197) + "..." : text;
    return { timestamp: "", type: "prompt", summary: truncated || "Prompt sent" };
  }

  // Result message
  if (type === "result") {
    return { timestamp: "", type: "result", summary: "Turn completed" };
  }

  return null;
}

function summarizeToolInput(name: string, input: Record<string, unknown>): string {
  switch (name) {
    case "Bash":
      return truncate(String(input.command || ""), 120);
    case "Read":
      return truncate(String(input.file_path || ""), 120);
    case "Write":
    case "Edit":
      return truncate(String(input.file_path || ""), 120);
    case "Glob":
      return truncate(String(input.pattern || ""), 120);
    case "Grep":
      return truncate(String(input.pattern || ""), 120);
    case "WebSearch":
      return truncate(String(input.query || ""), 120);
    case "Agent":
      return truncate(String(input.description || input.prompt || ""), 120);
    default:
      return "";
  }
}

function truncate(s: string, max: number): string {
  return s.length > max ? s.slice(0, max - 3) + "..." : s;
}

/**
 * Read the last `bytes` of a file efficiently.
 */
async function tailFile(path: string, bytes: number): Promise<string> {
  const info = await stat(path);
  const size = info.size;
  const start = Math.max(0, size - bytes);
  const fd = await open(path, "r");
  try {
    const buf = Buffer.alloc(Math.min(bytes, size));
    await fd.read(buf, 0, buf.length, start);
    return buf.toString("utf-8");
  } finally {
    await fd.close();
  }
}

export async function GET(request: NextRequest) {
  const missionDir = process.env.MISSION_DIR || process.cwd();
  const logPath = join(missionDir, "agent.log");

  const url = request.nextUrl;
  const tail = Math.min(parseInt(url.searchParams.get("tail") || "50", 10), 500);
  const after = url.searchParams.get("after") || "";

  try {
    // Read last 256KB of the log file (enough for hundreds of entries)
    const raw = await tailFile(logPath, 256 * 1024);
    const lines = raw.split("\n").filter(Boolean);

    const entries: LogEntry[] = [];

    for (const line of lines) {
      // Parse: [timestamp] {json}
      const match = line.match(/^\[(.+?)\]\s+(.+)$/);
      if (!match) continue;

      const timestamp = match[1];

      // Filter by after timestamp
      if (after && timestamp <= after) continue;

      try {
        const msg = JSON.parse(match[2]);
        const entry = summarizeMessage(msg);
        if (entry) {
          entry.timestamp = timestamp;
          entries.push(entry);
        }
      } catch {
        // Skip malformed JSON lines
      }
    }

    // Return last N entries
    const result = entries.slice(-tail);

    return NextResponse.json({ entries: result });
  } catch (err: unknown) {
    // File doesn't exist yet — normal when agent hasn't started
    if ((err as NodeJS.ErrnoException).code === "ENOENT") {
      return NextResponse.json({ entries: [] });
    }
    return NextResponse.json(
      { error: "Failed to read log" },
      { status: 500 }
    );
  }
}
