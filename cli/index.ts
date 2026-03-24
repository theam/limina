import { Command } from "commander";
import { existsSync, readFileSync } from "fs";
import { join } from "path";
import { init } from "./init";
import { start } from "./start";
import { stop } from "./stop";
import { status } from "./status";
import { budget } from "./budget";

/**
 * Auto-detect mission state and run the appropriate command.
 *
 * No mission.json        → init
 * mission.json + no daemon → start
 * mission.json + daemon    → status
 */
async function auto() {
  const cwd = process.cwd();
  const missionPath = join(cwd, "mission.json");
  const pidFile = join(cwd, ".limina.pid");

  // No mission — start from scratch
  if (!existsSync(missionPath)) {
    return init();
  }

  // Mission exists — check if daemon is running
  if (existsSync(pidFile)) {
    const mainPid = parseInt(readFileSync(pidFile, "utf-8").trim().split("\n")[0], 10);
    try {
      process.kill(mainPid, 0); // signal 0 = check existence
      // Daemon is alive — show status
      return status();
    } catch {
      // Stale PID file — daemon is dead, start it
    }
  }

  // Mission exists but daemon not running — start it
  return start({ port: "3000", open: true });
}

const program = new Command();

program
  .name("limina")
  .description("Cross the threshold between known and unknown")
  .version("2.0.0")
  .action(auto);

program
  .command("init")
  .description("Initialize a new research mission")
  .action(init);

program
  .command("start")
  .description("Start the research daemon (agent + observatory)")
  .option("-p, --port <port>", "Observatory port", "3000")
  .option("--no-open", "Don't open browser automatically")
  .action(start);

program
  .command("stop")
  .description("Stop the research daemon")
  .action(stop);

program
  .command("status")
  .description("Show mission status")
  .action(status);

program
  .command("budget [amount]")
  .description("View or set mission budget")
  .action(budget);

program.parse();
