import * as p from "@clack/prompts";
import color from "chalk";
import { existsSync, readFileSync } from "fs";
import { join } from "path";
import { readdir } from "fs/promises";

const blue = color.hex("#0f62fe");
const green = color.hex("#198038");
const yellow = color.hex("#f1c21b");
const red = color.hex("#da1e28");
const dim = color.hex("#525252");
const bright = color.hex("#161616");

export async function status() {
  const cwd = process.cwd();
  const missionPath = join(cwd, "mission.json");

  if (!existsSync(missionPath)) {
    p.intro(blue("Limina"));
    p.cancel("No research mission in this directory.");
    return;
  }

  const state = JSON.parse(readFileSync(missionPath, "utf-8"));

  console.log();
  p.intro(blue("Mission Status"));

  // Phase badge
  const phaseColor =
    state.phase === "COMPLETED" ? green :
    state.phase === "RUNNING" ? blue :
    state.phase === "FAILED_RECOVERABLE" || state.phase === "KILLED" ? red :
    state.phase === "STALLED" ? yellow : dim;

  // Elapsed
  const elapsed = state.startedAt ? formatElapsed(new Date(state.startedAt)) : "not started";

  // Artifact counts
  const kbPath = join(cwd, "kb");
  const counts = {
    H: await countFiles(join(kbPath, "research/hypotheses")),
    E: await countFiles(join(kbPath, "research/experiments")),
    F: await countFiles(join(kbPath, "research/findings")),
    T: await countFiles(join(kbPath, "tasks")),
    CR: await countFiles(join(kbPath, "reports"), "CR"),
  };

  // Daemon status
  const pidFile = join(cwd, ".limina.pid");
  const daemonRunning = existsSync(pidFile);

  const shortObj = (state.objective || "").length > 55
    ? state.objective.slice(0, 55) + "..."
    : state.objective || "Unknown";

  p.note(
    [
      `${bright("Phase:")}     ${phaseColor(state.phase)}`,
      `${bright("Objective:")} ${dim(shortObj)}`,
      `${bright("Elapsed:")}   ${dim(elapsed)}`,
      `${bright("Budget:")}    ${dim(state.budget || "not set")}`,
      "",
      `${bright("Artifacts:")}`,
      `  ${blue("H:" + counts.H)}  ${yellow("E:" + counts.E)}  ${green("F:" + counts.F)}  ${dim("T:" + counts.T)}  CR:${counts.CR}`,
      "",
      `${bright("Daemon:")}    ${daemonRunning ? green("running") : dim("stopped")}`,
    ].join("\n"),
    "Research Mission"
  );

  p.outro(dim("Run ") + bright("limina start") + dim(" to begin"));
}

async function countFiles(dir: string, prefix?: string): Promise<number> {
  try {
    const files = await readdir(dir);
    return files.filter((f) =>
      f.endsWith(".md") && (!prefix || f.startsWith(prefix))
    ).length;
  } catch {
    return 0;
  }
}

function formatElapsed(since: Date): string {
  const ms = Date.now() - since.getTime();
  const totalMinutes = Math.floor(ms / 60000);
  if (totalMinutes < 1) return "< 1 min";
  if (totalMinutes < 60) return `${totalMinutes}m`;
  const hours = Math.floor(totalMinutes / 60);
  const mins = totalMinutes % 60;
  if (hours < 24) return mins > 0 ? `${hours}h ${mins}m` : `${hours}h`;
  const days = Math.floor(hours / 24);
  return `${days}d ${hours % 24}h`;
}
