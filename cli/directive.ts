import * as p from "@clack/prompts";
import color from "chalk";
import { existsSync, readFileSync } from "fs";
import { join } from "path";

const green = color.hex("#198038");
const red = color.hex("#da1e28");
const dim = color.dim;
const bright = color.bold;

export async function directive(instruction?: string) {
  const cwd = process.cwd();
  const pidFile = join(cwd, ".limina.pid");

  // Find the running observatory port
  let port = 3000;
  if (existsSync(pidFile)) {
    // Try common ports
    for (const p of [3000, 3001, 3002, 3003]) {
      try {
        const res = await fetch(`http://localhost:${p}/api/status`);
        if (res.ok) {
          port = p;
          break;
        }
      } catch {}
    }
  }

  // If no instruction provided as argument, prompt for it
  if (!instruction) {
    const input = await p.text({
      message: "What should the agent do?",
      placeholder: "e.g., Focus on latency, stop investigating H003, try a different approach...",
      validate: (val) => {
        if (!val.trim()) return "Instruction cannot be empty";
      },
    });

    if (p.isCancel(input)) {
      p.cancel("Cancelled.");
      process.exit(0);
    }

    instruction = input as string;
  }

  // Send via API
  try {
    const res = await fetch(`http://localhost:${port}/api/directive`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ instruction, priority: "NORMAL" }),
    });

    if (res.ok) {
      const data = await res.json();
      console.log(green("  ✓ ") + `Directive sent (${data.id})`);
      console.log(dim("    The agent will incorporate this at the next phase boundary."));
    } else {
      const err = await res.json().catch(() => ({ error: "Unknown error" }));
      console.log(red("  ✗ ") + `Failed: ${err.error}`);
    }
  } catch {
    // Observatory not running — write directly to DIRECTIVES.md
    console.log(dim("  Observatory not running — writing directive to file."));
    try {
      const { writeFileSync } = require("fs");
      const { parseDirectives } = require("../src/lib/kb-parser");

      const directivesPath = join(cwd, "kb/mission/DIRECTIVES.md");
      let content = "";
      try {
        content = readFileSync(directivesPath, "utf-8");
      } catch {
        content = "# CEO Directives\n\n_Strategic instructions from the CEO to incorporate into ongoing work._\n";
      }

      const existing = parseDirectives(content);
      const maxNum = existing.reduce((max: number, d: { id: string }) => {
        const num = parseInt(d.id.replace("DIR-", ""), 10);
        return num > max ? num : max;
      }, 0);
      const nextId = `DIR-${String(maxNum + 1).padStart(3, "0")}`;

      const titleLine = instruction.trim().split("\n")[0];
      const title = titleLine.length > 60 ? titleLine.slice(0, 57) + "..." : titleLine;

      const entry = `\n## ${nextId}: ${title}\n> **Priority**: NORMAL\n> **Submitted**: ${new Date().toISOString()}\n> **Status**: PENDING\n\n${instruction.trim()}\n`;

      writeFileSync(directivesPath, content + entry);
      console.log(green("  ✓ ") + `Directive saved (${nextId})`);
      console.log(dim("    The agent will see it when it starts the next iteration."));
    } catch (err) {
      console.log(red("  ✗ ") + `Failed to write directive: ${err instanceof Error ? err.message : err}`);
    }
  }
}
