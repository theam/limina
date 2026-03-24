import * as p from "@clack/prompts";
import color from "chalk";
import chalkAnimation from "chalk-animation";
import gradientString from "gradient-string";
import { mkdir, writeFile, copyFile, readdir } from "fs/promises";
import { join } from "path";
import { existsSync, readFileSync, writeFileSync } from "fs";
import { execSync } from "child_process";
import { setTimeout as sleep } from "timers/promises";
import {
  runInitPreflight,
  installers,
  checkClaudeCli,
  checkClaudeAuth,
  type CheckResult,
} from "./preflight";

// DESIGN.md colors
const blue = color.hex("#0f62fe");
const green = color.hex("#198038");
const red = color.hex("#da1e28");
const dim = color.dim;
const bright = color.bold;

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

  // Pre-flight: check and auto-install prerequisites
  {
    const preflight = runInitPreflight();

    // Show passing checks
    for (const check of preflight.checks.filter((c) => c.ok)) {
      console.log(green("  ✓ ") + dim(check.name + " " + check.message));
    }

    if (!preflight.ok) {
      const missing = preflight.checks.filter(
        (c) => !c.ok && c.severity === "critical",
      );
      for (const check of missing) {
        console.log(red("  ✗ ") + check.name + " — " + dim(check.message));
      }

      const installable = missing.filter((c) => c.installable);
      if (installable.length > 0) {
        console.log();
        const shouldInstall = await p.confirm({
          message: `Install ${installable.length} missing prerequisite${installable.length > 1 ? "s" : ""}?`,
          initialValue: true,
        });

        if (shouldInstall && !p.isCancel(shouldInstall)) {
          for (const check of installable) {
            if (check.name === "Claude auth") {
              const claudeCheck = checkClaudeCli();
              if (!claudeCheck.ok) continue;
              console.log();
              console.log(blue("  ◇ ") + "Claude Code needs authentication");
              console.log();
              console.log(dim("    Run this in another terminal:"));
              console.log();
              console.log("      " + bright("claude auth login"));
              console.log();
              await p.text({
                message: "Press Enter when done",
                defaultValue: "",
                placeholder: "",
              });
              const recheck = checkClaudeAuth();
              if (recheck.ok) {
                console.log(green("  ✓ ") + "Authenticated as " + dim(recheck.message));
              } else {
                console.log(red("  ✗ ") + "Still not authenticated — you can continue later with " + bright("limina doctor"));
              }
              continue;
            }

            const installer = installers[check.name];
            if (!installer) continue;
            const s = p.spinner();
            s.start(`Installing ${check.name}`);
            const result = installer();
            if (result.ok) {
              s.stop(green("✓") + ` ${check.name} installed`);
            } else {
              s.stop(red("✗") + ` ${check.name} install failed`);
              if (check.fix) console.log(dim("    → ") + check.fix);
            }
          }
        }
      }

      // Re-check after install attempts
      const recheck = runInitPreflight();
      if (!recheck.ok) {
        const still = recheck.checks.filter(
          (c) => !c.ok && c.severity === "critical",
        );
        console.log();
        for (const check of still) {
          console.log(red("  ✗ ") + check.name + " — " + dim(check.message));
          if (check.fix) console.log(dim("    → ") + check.fix);
        }
        p.cancel(
          "Fix the issues above, or run " +
            color.bold("limina doctor") +
            " for a full health check.",
        );
        process.exit(1);
      }
    }

    console.log();
  }

  // Prompt for Anthropic API key if not already set
  if (!process.env.ANTHROPIC_API_KEY) {
    console.log();
    console.log(blue("  ◇ ") + "Limina needs an Anthropic API key to run the research agent.");
    console.log(dim("    Get one at: ") + bright("https://console.anthropic.com/settings/keys"));
    console.log();

    const apiKey = await p.text({
      message: "Paste your Anthropic API key",
      placeholder: "sk-ant-...",
      validate: (val) => {
        if (!val.trim()) return "API key is required";
        if (!val.startsWith("sk-ant-")) return "Key should start with sk-ant-";
      },
    });

    if (p.isCancel(apiKey)) {
      p.cancel("Setup cancelled.");
      process.exit(0);
    }

    if (apiKey) {
      const cwd = process.cwd();
      const envPath = join(cwd, ".env.local");
      let envContent = "";
      try {
        envContent = existsSync(envPath) ? readFileSync(envPath, "utf-8") : "";
      } catch {}

      if (envContent.includes("ANTHROPIC_API_KEY=")) {
        envContent = envContent.replace(
          /ANTHROPIC_API_KEY=.*/,
          `ANTHROPIC_API_KEY=${apiKey}`
        );
      } else {
        envContent += `${envContent && !envContent.endsWith("\n") ? "\n" : ""}ANTHROPIC_API_KEY=${apiKey}\n`;
      }
      writeFileSync(envPath, envContent);
      process.env.ANTHROPIC_API_KEY = apiKey;
      console.log(green("  ✓ ") + "API key saved to " + dim(".env.local"));
    }
  } else {
    console.log(green("  ✓ ") + dim("Anthropic API key found"));
  }

  // Model selection
  if (!process.env.LIMINA_MODEL) {
    console.log();
    const model = await p.select({
      message: "Which model should the research agent use?",
      options: [
        {
          value: "claude-opus-4-6",
          label: "Opus — best for research (recommended)",
          hint: "deepest reasoning, highest quality results",
        },
        {
          value: "claude-sonnet-4-6",
          label: "Sonnet — cost-effective",
          hint: "faster and cheaper, good for exploratory work",
        },
      ],
    });

    if (p.isCancel(model)) {
      p.cancel("Setup cancelled.");
      process.exit(0);
    }

    const cwd = process.cwd();
    const envPath = join(cwd, ".env.local");
    let envContent = "";
    try {
      envContent = existsSync(envPath) ? readFileSync(envPath, "utf-8") : "";
    } catch {}
    envContent += `${envContent && !envContent.endsWith("\n") ? "\n" : ""}LIMINA_MODEL=${model}\n`;
    writeFileSync(envPath, envContent);
    process.env.LIMINA_MODEL = model as string;
    console.log(green("  ✓ ") + "Model set to " + dim(model as string));
  } else {
    console.log(green("  ✓ ") + dim(`Model: ${process.env.LIMINA_MODEL}`));
  }

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

  // Step 1: Objective — the most important question, show examples
  console.log();
  console.log(dim("  Examples:"));
  console.log(dim("  • \"Find the best embedding model for Spanish legal documents\""));
  console.log(dim("  • \"Compare RAG vs fine-tuning for our customer support bot\""));
  console.log(dim("  • \"Why is our search relevance dropping on long queries?\""));
  console.log();

  const objective = await p.text({
    message: "What should the agent research?",
    placeholder: "e.g., Find the fastest way to serve our ML model under 100ms",
    validate: (val) => {
      if (!val.trim()) return "Tell the agent what to research";
    },
  });

  if (p.isCancel(objective)) {
    p.cancel("Setup cancelled.");
    process.exit(0);
  }

  // Step 2: Context — optional, explain what it's for
  console.log();
  console.log(dim("  Give the agent a head start — what has been tried, what exists today."));
  console.log(dim("  Example: \"We use pgvector with OpenAI embeddings. Retrieval is slow on 1M+ rows.\""));
  console.log();

  const context = await p.text({
    message: "Any context the agent should know? " + dim("(Enter to skip)"),
    placeholder: "e.g., We currently use X, we tried Y but it didn't work because Z",
    defaultValue: "",
  });

  if (p.isCancel(context)) {
    p.cancel("Setup cancelled.");
    process.exit(0);
  }

  // Step 3: Success metric — optional with smart default
  console.log();
  console.log(dim("  How will you know the research succeeded? A number, a comparison, a clear answer."));
  console.log(dim("  Example: \"Latency under 200ms at p95\" or \"A ranked comparison of 3+ approaches\""));
  console.log();

  const successMetric = await p.text({
    message: "What does success look like? " + dim("(Enter for default)"),
    placeholder: "e.g., A clear recommendation backed by experimental evidence",
    defaultValue: "A clear recommendation with evidence from at least 2 experiments",
  });

  if (p.isCancel(successMetric)) {
    p.cancel("Setup cancelled.");
    process.exit(0);
  }

  // Step 4: Repository (optional)
  const repository = await p.text({
    message: "Repository to target " + dim("(Enter to skip)"),
    placeholder: "e.g., github.com/your-org/your-repo",
    defaultValue: "",
  });

  if (p.isCancel(repository)) {
    p.cancel("Setup cancelled.");
    process.exit(0);
  }

  // Step 5: Budget
  const maxBudget = await p.text({
    message: "Max budget " + dim("(API costs — Enter for $150)"),
    placeholder: "$150",
    defaultValue: "$150",
  });

  if (p.isCancel(maxBudget)) {
    p.cancel("Setup cancelled.");
    process.exit(0);
  }

  // Step 6: Slack notifications (optional)
  const wantSlack = await p.confirm({
    message: "Notify you on Slack when the agent needs input?",
    initialValue: false,
  });

  let slackWebhook = "";
  if (wantSlack && !p.isCancel(wantSlack)) {
    const webhook = await p.text({
      message: "Slack webhook URL",
      placeholder: "https://hooks.slack.com/services/T00.../B00.../xxx",
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
    const filesToCopy = ["CLAUDE.md", "AGENTS.md", "LIMINA.md"];
    for (const file of filesToCopy) {
      const src = join(frameworkDir, file);
      if (existsSync(src)) {
        await copyFile(src, join(cwd, file));
      }
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

  p.outro(green("Your research mission is ready") + dim(" — starting agent..."));
}
