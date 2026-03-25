import * as p from "@clack/prompts";
import color from "chalk";
import { existsSync, writeFileSync, readFileSync, createWriteStream } from "fs";
import { join } from "path";
import { spawn, exec } from "child_process";
import { createServer } from "net";
import { setTimeout as sleep } from "timers/promises";
import {
  runStartPreflight,
  installers,
  checkClaudeCli,
  checkClaudeAuth,
} from "./preflight";

const blue = color.hex("#0f62fe");
const green = color.hex("#198038");
const red = color.hex("#da1e28");
const yellow = color.hex("#f1c21b");
const dim = color.dim;
const bright = color.bold;

function getFreePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.once("listening", () => {
      const addr = server.address();
      const port = typeof addr === "object" && addr ? addr.port : 0;
      server.close(() => resolve(port));
    });
    server.listen(0, "127.0.0.1");
  });
}

export async function start(options: { port?: string; open?: boolean }) {
  const cwd = process.cwd();

  console.clear();
  console.log();

  p.intro(blue("Starting research daemon"));

  // Validate project
  if (!existsSync(join(cwd, "kb/mission/CHALLENGE.md"))) {
    p.cancel(
      "No research mission found.\n" +
        dim("  Run ") +
        bright("limina init") +
        dim(" to create one.")
    );
    process.exit(1);
  }

  // Check for stale PID
  const pidFile = join(cwd, ".limina.pid");
  if (existsSync(pidFile)) {
    const oldPid = parseInt(readFileSync(pidFile, "utf-8").trim().split("\n")[0], 10);
    try {
      process.kill(oldPid, 0);
      p.cancel(
        `Daemon already running (PID ${oldPid}).\n` +
          dim("  Run ") +
          bright("limina stop") +
          dim(" first.")
      );
      process.exit(1);
    } catch {
      // Stale — clean up
    }
  }

  // Pre-flight: check and auto-install prerequisites
  const s = p.spinner();
  s.start("Checking prerequisites");

  const preflight = runStartPreflight();

  if (!preflight.ok) {
    const missing = preflight.checks.filter((c) => !c.ok);
    s.stop(
      red("✗") +
        ` ${preflight.criticalFailures} issue${preflight.criticalFailures > 1 ? "s" : ""} found`,
    );
    console.log();
    for (const check of missing) {
      const icon = check.severity === "critical" ? red("✗") : yellow("⚠");
      console.log(`  ${icon} ${check.name} — ${dim(check.message)}`);
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
              console.log(red("  ✗ ") + "Still not authenticated — run " + bright("claude auth login") + " and try again");
            }
            continue;
          }

          const installer = installers[check.name];
          if (!installer) continue;
          const installSpinner = p.spinner();
          installSpinner.start(`Installing ${check.name}`);
          const result = installer();
          if (result.ok) {
            installSpinner.stop(green("✓") + ` ${check.name} installed`);
          } else {
            installSpinner.stop(red("✗") + ` ${check.name} install failed`);
            if (check.fix) console.log(dim("    → ") + check.fix);
          }
        }
      }

      // Re-check
      const recheck = runStartPreflight();
      if (!recheck.ok) {
        p.cancel(
          "Fix the issues above, or run " +
            color.bold("limina doctor") +
            " for details.",
        );
        process.exit(1);
      }
    } else {
      p.cancel(
        "Fix the issues above, or run " +
          color.bold("limina doctor") +
          " for details.",
      );
      process.exit(1);
    }

    s.start("Prerequisites OK");
    s.stop(green("✓") + " All prerequisites OK");
  } else {
    s.stop(green("✓") + " All prerequisites OK");
  }

  await sleep(200);

  // Check for Anthropic API key
  if (!process.env.ANTHROPIC_API_KEY) {
    console.log();
    console.log(blue("  ◇ ") + "Anthropic API key not found.");
    console.log(dim("    Get one at: ") + bright("https://console.anthropic.com/settings/keys"));
    console.log();

    const apiKey = await p.text({
      message: "Paste your Anthropic API key",
      placeholder: "sk-ant-...",
      validate: (val) => {
        if (!val.trim()) return "API key is required to run the agent";
        if (!val.startsWith("sk-ant-")) return "Key should start with sk-ant-";
      },
    });

    if (p.isCancel(apiKey)) {
      p.cancel("Cannot start without an API key.");
      process.exit(1);
    }

    // Save to project's .env.local
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
    process.env.ANTHROPIC_API_KEY = apiKey as string;
    console.log(green("  ✓ ") + "API key saved to " + dim(".env.local"));
  }

  // Model selection (if not already set)
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
      p.cancel("Cannot start without a model selection.");
      process.exit(1);
    }

    const envPath = join(cwd, ".env.local");
    let envContent = "";
    try {
      envContent = existsSync(envPath) ? readFileSync(envPath, "utf-8") : "";
    } catch {}
    envContent += `${envContent && !envContent.endsWith("\n") ? "\n" : ""}LIMINA_MODEL=${model}\n`;
    writeFileSync(envPath, envContent);
    process.env.LIMINA_MODEL = model as string;
    console.log(green("  ✓ ") + "Model set to " + dim(model as string));
  }

  // Read mission info
  const missionPath = join(cwd, "mission.json");
  let missionObj = "";
  if (existsSync(missionPath)) {
    const state = JSON.parse(readFileSync(missionPath, "utf-8"));
    missionObj = state.objective || "";
    if (state.phase === "CREATED") {
      state.phase = "RUNNING";
      state.startedAt = new Date().toISOString();
      writeFileSync(missionPath, JSON.stringify(state, null, 2));
    }
  }

  // Find available ports for both services
  const s2 = p.spinner();
  s2.start("Starting services");

  const uiPort = options.port ? parseInt(options.port, 10) : await getFreePort();
  const agentPort = await getFreePort();

  const observatoryDir = join(__dirname, "..");
  const nextBin = join(observatoryDir, "node_modules/.bin/next");
  const tsxBin = join(observatoryDir, "node_modules/.bin/tsx");
  const agentServicePath = join(observatoryDir, "src", "agent-service", "server.ts");

  // Log files for both services
  const uiLogPath = join(cwd, "ui-server.log");
  const agentLogPath = join(cwd, "agent-service.log");
  const uiLogStream = createWriteStream(uiLogPath, { flags: "a" });
  const agentLogStream = createWriteStream(agentLogPath, { flags: "a" });

  // Start agent service first (Next.js needs its URL)
  const agentService = spawn(tsxBin, [agentServicePath], {
    cwd: observatoryDir,
    env: { ...process.env, AGENT_PORT: String(agentPort), MISSION_DIR: cwd },
    stdio: ["ignore", "pipe", "pipe"],
  });
  agentService.stdout?.pipe(agentLogStream);
  agentService.stderr?.pipe(agentLogStream);

  // Wait for agent service to be ready
  const waitForAgentService = async () => {
    for (let i = 0; i < 20; i++) {
      try {
        const res = await fetch(`http://127.0.0.1:${agentPort}/health`);
        if (res.ok) return true;
      } catch {}
      await sleep(500);
    }
    return false;
  };

  const agentReady = await waitForAgentService();
  if (!agentReady) {
    agentService.kill();
    p.cancel("Agent service failed to start. Check logs.");
    process.exit(1);
  }

  // Start Next.js UI server with agent service URL
  const agentServiceUrl = `http://127.0.0.1:${agentPort}`;

  const webServer = spawn(nextBin, ["dev", "--port", String(uiPort)], {
    cwd: observatoryDir,
    env: { ...process.env, MISSION_DIR: cwd, PORT: String(uiPort), AGENT_SERVICE_URL: agentServiceUrl },
    stdio: ["ignore", "pipe", "pipe"],
  });
  webServer.stdout?.pipe(uiLogStream);
  webServer.stderr?.pipe(uiLogStream);

  s2.stop(green("✓") + " Observatory on " + color.underline(`http://localhost:${uiPort}`));

  await sleep(200);

  // Write PID file (CLI pid, web server pid, agent service pid)
  writeFileSync(pidFile, `${process.pid}\n${webServer.pid}\n${agentService.pid}`);

  await sleep(300);

  // Mission summary
  const shortObj = missionObj.length > 55 ? missionObj.slice(0, 55) + "..." : missionObj;
  p.note(
    [
      `${bright("Mission:")}  ${dim(shortObj)}`,
      `${bright("Agent:")}    ${dim("Claude Code (autonomous)")}`,
      `${bright("View:")}     ${color.underline(dim(`http://localhost:${uiPort}`))}`,
      "",
      dim("The agent is now researching autonomously."),
      dim("Open the observatory to monitor progress."),
      "",
      dim("Press ") + bright("Ctrl+C") + dim(" to stop."),
    ].join("\n"),
    "Running"
  );

  // Wait for web server ready, then start agent and open browser
  const s3 = p.spinner();
  s3.start("Launching research agent");

  const waitForServer = async () => {
    for (let i = 0; i < 20; i++) {
      try {
        await fetch(`http://localhost:${uiPort}`);
        return true;
      } catch {
        await sleep(500);
      }
    }
    return false;
  };

  const ready = await waitForServer();
  if (ready) {
    // Start the agent via the web server's API (proxied to agent service)
    try {
      const apiKey = process.env.MISSION_API_KEY || "";
      await fetch(`http://localhost:${uiPort}/api/agent/start`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "Authorization": `Bearer ${apiKey}`,
        },
        body: JSON.stringify({ phase: "running" }),
      });
    } catch {
      // Agent start via API failed — may need to be started from the UI
    }

    if (options.open !== false) {
      const openCmd =
        process.platform === "darwin" ? "open" :
        process.platform === "win32" ? "start" : "xdg-open";
      exec(`${openCmd} http://localhost:${uiPort}`);
    }
  }

  s3.stop(green("✓") + " Research agent running");

  p.outro(green("Research in progress") + dim(" — observatory at localhost:" + uiPort));

  // Keep alive — kill both services on shutdown
  const shutdown = (signal: string) => {
    if (signal === "SIGINT") {
      console.log();
      console.log(dim("  Shutting down..."));
    }
    agentService.kill();
    webServer.kill();
    try { require("fs").unlinkSync(pidFile); } catch {}
    if (signal === "SIGINT") {
      console.log(green("  ✓") + " Stopped. Research state preserved.");
      console.log();
    }
    process.exit(0);
  };

  process.on("SIGINT", () => shutdown("SIGINT"));
  process.on("SIGTERM", () => shutdown("SIGTERM"));
}
