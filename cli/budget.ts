import * as p from "@clack/prompts";
import color from "chalk";
import { existsSync, readFileSync, writeFileSync } from "fs";
import { join } from "path";

const blue = color.hex("#0f62fe");
const green = color.hex("#198038");
const dim = color.dim;
const bright = color.bold;

export async function budget(amount?: string) {
  const missionPath = join(process.cwd(), "mission.json");

  if (!existsSync(missionPath)) {
    p.intro(blue("Limina"));
    p.cancel("No research mission in this directory.");
    return;
  }

  const state = JSON.parse(readFileSync(missionPath, "utf-8"));

  console.log();

  if (!amount) {
    p.intro(blue("Mission Budget"));
    p.note(
      `${bright("Current budget:")} ${dim(state.budget || "not set")}`,
      "Budget"
    );
    p.outro(dim("Set a new budget: ") + bright("limina budget $200"));
    return;
  }

  p.intro(blue("Updating Budget"));

  state.budget = amount.startsWith("$") ? amount : `$${amount}`;
  writeFileSync(missionPath, JSON.stringify(state, null, 2));

  p.outro(green("✓") + ` Budget updated to ${bright(state.budget)}`);
}
