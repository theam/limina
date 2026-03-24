import { Command } from "commander";
import { init } from "./init";
import { start } from "./start";
import { stop } from "./stop";
import { status } from "./status";
import { budget } from "./budget";

const program = new Command();

program
  .name("limina")
  .description("Cross the threshold between known and unknown")
  .version("2.0.0");

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
