import * as p from "@clack/prompts";
import color from "chalk";
import { existsSync, readFileSync, unlinkSync } from "fs";
import { join } from "path";

const blue = color.hex("#0f62fe");
const green = color.hex("#198038");
const dim = color.dim;

export async function stop() {
  console.log();
  p.intro(blue("Stopping research daemon"));

  const pidFile = join(process.cwd(), ".limina.pid");

  if (!existsSync(pidFile)) {
    p.cancel("No daemon running in this directory.");
    return;
  }

  const s = p.spinner();
  s.start("Stopping processes");

  const pids = readFileSync(pidFile, "utf-8").trim().split("\n").map(Number);

  for (const pid of pids) {
    try {
      process.kill(pid, "SIGTERM");
    } catch {
      // Already dead
    }
  }

  unlinkSync(pidFile);

  s.stop(green("✓") + " Daemon stopped");

  p.outro(green("Research state preserved") + dim(" — restart anytime with ") + color.bold("start"));
}
