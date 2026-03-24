import * as p from "@clack/prompts";
import color from "chalk";
import chalkAnimation from "chalk-animation";
import gradientString from "gradient-string";
import { mkdir, writeFile, copyFile, readdir } from "fs/promises";
import { join } from "path";
import { existsSync } from "fs";
import { execSync } from "child_process";
import { setTimeout as sleep } from "timers/promises";

// DESIGN.md colors
const blue = color.hex("#0f62fe");
const green = color.hex("#198038");
const dim = color.hex("#525252");
const bright = color.hex("#161616");

// Universe theme — deep space gradient
const cosmos = gradientString(["#0f62fe", "#6929c4", "#9f1853", "#0f62fe"]);
const nebula = gradientString(["#6929c4", "#0f62fe", "#005d5d", "#0f62fe"]);
const starfield = gradientString(["#525252", "#a8a8a8", "#ffffff", "#a8a8a8", "#525252"]);

export async function init() {
  console.clear();

  // Phase 1: The cosmos appears — animated title
  const title = `
  ✦  ·  ˚  ✧  .  ·  ✦  ˚  .  ✧  ·  ✦  ˚  .  ✧

   ██╗     ██╗███╗   ███╗██╗███╗   ██╗ █████╗
   ██║     ██║████╗ ████║██║████╗  ██║██╔══██╗
   ██║     ██║██╔████╔██║██║██╔██╗ ██║███████║
   ██║     ██║██║╚██╔╝██║██║██║╚██╗██║██╔══██║
   ███████╗██║██║ ╚═╝ ██║██║██║ ╚████║██║  ██║
   ╚══════╝╚═╝╚═╝     ╚═╝╚═╝╚═╝  ╚═══╝╚═╝  ╚═╝

  ✧  .  ·  ✦  ˚  .  ✧  ·  ✦  ˚  .  ✧  ·  ✦
`;

  // Animate the title with cosmic colors
  const animation = chalkAnimation.karaoke(title, 0.5);
  await sleep(2500);
  animation.stop();

  // Replace with static styled version
  console.clear();
  console.log();
  console.log(starfield("    ✦  ·  ˚  ✧  .  ·  ✦  ˚  .  ✧  ·  ✦"));
  console.log();
  // Name
  console.log(cosmos("        ██╗     ██╗███╗   ███╗██╗███╗   ██╗ █████╗ "));
  console.log(cosmos("        ██║     ██║████╗ ████║██║████╗  ██║██╔══██╗"));
  console.log(cosmos("        ██║     ██║██╔████╔██║██║██╔██╗ ██║███████║"));
  console.log(cosmos("        ██║     ██║██║╚██╔╝██║██║██║╚██╗██║██╔══██║"));
  console.log(cosmos("        ███████╗██║██║ ╚═╝ ██║██║██║ ╚████║██║  ██║"));
  console.log(cosmos("        ╚══════╝╚═╝╚═╝     ╚═╝╚═╝╚═╝  ╚═══╝╚═╝  ╚═╝"));
  console.log();
  console.log(dim("      The AI agent will research autonomously and report findings."));
  console.log(dim("      You'll monitor progress and steer direction from the observatory."));
  console.log();
  console.log(starfield("    ✧  .  ·  ✦  ˚  .  ✧  ·  ✦  ˚  .  ✧  ·  ✦"));
  console.log();

  await sleep(1000);

  p.intro(blue("Set up your research mission"));

  // Check if already initialized
  const cwd = process.cwd();
  if (existsSync(join(cwd, "kb"))) {
    p.cancel("This directory already has a research mission. Use a new directory.");
    process.exit(1);
  }

  await sleep(300);

  const template = "research";

  await sleep(300);

  // Step 1: Objective
  const objective = await p.text({
    message: "What problem are you investigating?",
    placeholder: "Describe your research objective",
  });

  if (p.isCancel(objective)) {
    p.cancel("Setup cancelled.");
    process.exit(0);
  }

  // Step 2: Context
  const context = await p.text({
    message: "What does your current system look like?",
    placeholder: "Describe your baseline and what you've tried",
  });

  if (p.isCancel(context)) {
    p.cancel("Setup cancelled.");
    process.exit(0);
  }

  // Step 3: Success metric
  const successMetric = await p.text({
    message: "How will you measure success?",
    placeholder: "e.g., Recall@10 improvement ≥ 5%",
  });

  if (p.isCancel(successMetric)) {
    p.cancel("Setup cancelled.");
    process.exit(0);
  }

  // Step 4: Repository (optional)
  const repository = await p.text({
    message: "Repository to target " + dim("(optional, press Enter to skip)"),
    placeholder: "github.com/org/repo",
    defaultValue: "",
  });

  if (p.isCancel(repository)) {
    p.cancel("Setup cancelled.");
    process.exit(0);
  }

  // Step 5: Budget
  const maxBudget = await p.text({
    message: "Max budget for this mission",
    placeholder: "$150",
    defaultValue: "$150",
  });

  if (p.isCancel(maxBudget)) {
    p.cancel("Setup cancelled.");
    process.exit(0);
  }

  // Step 6: Slack notifications (optional)
  const wantSlack = await p.confirm({
    message: "Enable Slack notifications for agent requests?",
    initialValue: false,
  });

  let slackWebhook = "";
  if (wantSlack && !p.isCancel(wantSlack)) {
    const webhook = await p.text({
      message: "Slack webhook URL",
      placeholder: "https://hooks.slack.com/services/...",
    });
    if (!p.isCancel(webhook)) {
      slackWebhook = webhook as string;
    }
  }

  // Scaffolding phase — the "installation" experience
  console.log();

  const s = p.spinner();
  s.start("Creating your research workspace");

  await sleep(600);

  // Create KB structure
  const kbDirs = [
    "mission",
    "tasks",
    "research/hypotheses",
    "research/experiments",
    "research/findings",
    "research/literature",
    "research/data",
    "engineering/features",
    "engineering/investigations",
    "engineering/implementations",
    "engineering/retrospectives",
    "reports",
  ];

  for (const dir of kbDirs) {
    await mkdir(join(cwd, "kb", dir), { recursive: true });
  }

  s.message("Writing mission brief");
  await sleep(400);

  // Generate CHALLENGE.md
  const templateIntro =
    "This is an autonomous research mission.";

  const challengeContent = `# Research Mission

${templateIntro}

## Objective

${objective}

## Context & Baseline

${context}
${repository ? `\nRepository: ${repository}` : ""}

## Success Metric

${successMetric}

## Constraints

- Autonomy level: Fully autonomous — escalate only when truly blocked
- Maximum runtime: 48h
- Budget: ${maxBudget}
- If evaluation data does not exist, generate it yourself and document how it was created.
- If additional tools, budget, or access are needed, ask with a clear justification via CEO_REQUESTS.md.

## Escalation Rules

When blocked on resources, access, or decisions, create an entry in \`kb/mission/CEO_REQUESTS.md\` with status PENDING. The system will notify the human operator. Do not proceed on blocked items — wait for a response.
`;

  await writeFile(join(cwd, "kb/mission/CHALLENGE.md"), challengeContent);
  await writeFile(
    join(cwd, "kb/mission/BACKLOG.md"),
    `# Research Backlog\n\n## Last IDs\n> T: T000\n\n## Tasks\n\n| ID | Title | Status | Priority | Type |\n|---|---|---|---|---|\n\n_The research agent will populate this backlog during execution._\n`
  );
  await writeFile(join(cwd, "kb/INDEX.md"), "# Knowledge Base Index\n\n_Auto-populated as research progresses._\n");
  await writeFile(join(cwd, "kb/mission/DECISIONS.md"), "# Decisions\n\n_Decisions will be recorded here._\n");
  await writeFile(join(cwd, "kb/mission/CEO_REQUESTS.md"), "# CEO Requests\n\n_Requests for human input will appear here._\n");
  await writeFile(join(cwd, "kb/mission/FEEDBACK.md"), "# Feedback\n\n_User feedback on findings and direction will appear here._\n");

  s.message("Setting up research framework");
  await sleep(500);

  // Write mission.json
  const budgetStr = (maxBudget as string).startsWith("$") ? maxBudget as string : `$${maxBudget}`;
  const missionState = {
    id: `m_${Date.now().toString(36)}`,
    phase: "CREATED",
    createdAt: new Date().toISOString(),
    template,
    autonomyLevel: "full",
    objective,
    budget: budgetStr,
    slackWebhook: slackWebhook || undefined,
    estimatedCost: "$20–$150",
    estimatedDuration: "8–48 hours",
  };
  await writeFile(join(cwd, "mission.json"), JSON.stringify(missionState, null, 2));

  // Copy framework files
  const frameworkDir = join(__dirname, "..", "framework");
  if (existsSync(frameworkDir)) {
    const filesToCopy = ["CLAUDE.md", "AGENTS.md", "COOK.md"];
    for (const file of filesToCopy) {
      const src = join(frameworkDir, file);
      if (existsSync(src)) {
        await copyFile(src, join(cwd, file));
      }
    }

    await mkdir(join(cwd, ".cook"), { recursive: true });
    const cookConfig = join(frameworkDir, ".cook/config.json");
    if (existsSync(cookConfig)) {
      await copyFile(cookConfig, join(cwd, ".cook/config.json"));
    }

    const templateDir = join(frameworkDir, "templates");
    if (existsSync(templateDir)) {
      await mkdir(join(cwd, "templates"), { recursive: true });
      const templates = await readdir(templateDir);
      for (const t of templates) {
        await copyFile(join(templateDir, t), join(cwd, "templates", t));
      }
    }

    const scriptsDir = join(frameworkDir, "scripts");
    if (existsSync(scriptsDir)) {
      await mkdir(join(cwd, "scripts"), { recursive: true });
      const scripts = await readdir(scriptsDir);
      for (const sc of scripts) {
        await copyFile(join(scriptsDir, sc), join(cwd, "scripts", sc));
      }
    }
  }

  s.message("Initializing version control");
  await sleep(400);

  // Git init
  try {
    if (!existsSync(join(cwd, ".git"))) {
      execSync("git init", { cwd, stdio: "ignore" });
    }
    execSync("git add -A && git commit -m 'Research mission initialized'", {
      cwd,
      stdio: "ignore",
    });
  } catch {
    // Non-fatal
  }

  s.stop(green("✓") + " Research workspace ready");

  await sleep(300);

  // Summary — the "you're ready" screen
  console.log();
  p.note(
    [
      `${bright("Mission:")}   ${dim(typeof objective === "string" ? (objective.length > 50 ? objective.slice(0, 50) + "..." : objective) : "")}`,
      `${bright("Type:")}      ${dim("Autonomous Research")}`,
      `${bright("Budget:")}    ${dim(budgetStr)}`,
      `${bright("Estimate:")}  ${dim(missionState.estimatedCost + " / " + missionState.estimatedDuration)}`,
      slackWebhook ? `${bright("Slack:")}     ${dim("enabled")}` : "",
    ]
      .filter(Boolean)
      .join("\n"),
    "Mission Summary"
  );

  await sleep(300);

  // Next steps — clear, guided
  console.log();
  console.log(
    blue("  Next step: ") +
      bright("limina start")
  );
  console.log();
  console.log(
    dim("  This will start the research agent and open the observatory")
  );
  console.log(
    dim("  at ") +
      color.underline(dim("http://localhost:3000")) +
      dim(" where you can monitor progress,")
  );
  console.log(
    dim("  review findings, and steer direction.")
  );
  console.log();

  p.outro(green("Your research mission is ready") + dim(" — run limina start to begin"));
}
