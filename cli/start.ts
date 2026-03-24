import * as p from "@clack/prompts";
import color from "chalk";
import { existsSync, writeFileSync, readFileSync } from "fs";
import { join } from "path";
import { spawn, exec } from "child_process";
import { setTimeout as sleep } from "timers/promises";

const blue = color.hex("#0f62fe");
const green = color.hex("#198038");
const red = color.hex("#da1e28");
const dim = color.hex("#525252");
const bright = color.hex("#161616");

export async function start(options: { port?: string; open?: boolean }) {
  const cwd = process.cwd();
  const port = options.port || "3000";

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

  // Check cook
  const s = p.spinner();
  s.start("Checking prerequisites");
  await sleep(400);

  try {
    const { execSync } = require("child_process");
    execSync("which cook", { stdio: "ignore" });
    s.stop(green("✓") + " cook CLI found");
  } catch {
    s.stop(red("✗") + " cook CLI not found");
    p.cancel(
      "Install cook to continue:\n" +
        dim("  npm install -g @let-it-cook/cli")
    );
    process.exit(1);
  }

  await sleep(200);

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

  // Start observatory
  const s2 = p.spinner();
  s2.start("Starting observatory");
  await sleep(500);

  const observatoryDir = join(__dirname, "..");
  const nextBin = join(observatoryDir, "node_modules/.bin/next");

  const webServer = spawn(nextBin, ["dev", "--port", port], {
    cwd: observatoryDir,
    env: { ...process.env, MISSION_DIR: cwd, PORT: port },
    stdio: ["ignore", "pipe", "pipe"],
  });

  s2.stop(green("✓") + " Observatory on " + color.underline(`http://localhost:${port}`));

  await sleep(200);

  // Start research agent
  const s3 = p.spinner();
  s3.start("Launching research agent");
  await sleep(600);

  const cookProc = spawn(
    "cook",
    [
      "Continue research",
      "review",
      "Review current status and verify if we achieved the target mission",
      "DONE if we achieved the target mission, else ITERATE",
      "--agent", "claude",
    ],
    {
      cwd,
      detached: true,
      stdio: ["ignore", "pipe", "pipe"],
      env: { ...process.env },
    }
  );

  // Write PID file
  writeFileSync(pidFile, `${process.pid}\n${webServer.pid}\n${cookProc.pid}`);

  // Log agent output
  const logFile = join(cwd, "agent.log");
  const { appendFileSync } = require("fs");
  cookProc.stdout?.on("data", (data: Buffer) => {
    appendFileSync(logFile, `[${new Date().toISOString()}] ${data}`);
  });
  cookProc.stderr?.on("data", (data: Buffer) => {
    appendFileSync(logFile, `[${new Date().toISOString()}] ERR: ${data}`);
  });

  cookProc.on("exit", (code: number | null) => {
    if (existsSync(missionPath)) {
      const state = JSON.parse(readFileSync(missionPath, "utf-8"));

      if (code === 0) {
        // Check if there are pending CEO requests — that means BLOCKED, not COMPLETED
        const ceoPath = join(cwd, "kb/mission/CEO_REQUESTS.md");
        let hasPendingRequests = false;
        try {
          const ceoContent = readFileSync(ceoPath, "utf-8");
          hasPendingRequests = /\*\*Status\*\*:\s*PENDING/i.test(ceoContent);
        } catch {}

        // Check if tasks are still BLOCKED or BACKLOG
        const backlogPath = join(cwd, "kb/mission/BACKLOG.md");
        let hasBlockedTasks = false;
        try {
          const backlog = readFileSync(backlogPath, "utf-8");
          hasBlockedTasks = /\|\s*BLOCKED\s*\|/.test(backlog);
        } catch {}

        if (hasPendingRequests || hasBlockedTasks) {
          state.phase = "BLOCKED";
          state.blockedReason = hasPendingRequests
            ? "Agent needs your input — check CEO Requests in the observatory"
            : "Tasks are blocked — check the Research tab for details";
        } else {
          state.phase = "COMPLETED";
          state.completedAt = new Date().toISOString();
        }
      } else {
        state.phase = "FAILED_RECOVERABLE";
        state.failureReason = `Agent exited with code ${code}`;
      }
      writeFileSync(missionPath, JSON.stringify(state, null, 2));
    }
  });

  s3.stop(green("✓") + " Research agent running");

  await sleep(300);

  // Mission summary
  const shortObj = missionObj.length > 55 ? missionObj.slice(0, 55) + "..." : missionObj;
  p.note(
    [
      `${bright("Mission:")}  ${dim(shortObj)}`,
      `${bright("Agent:")}    ${dim("Claude Code (autonomous)")}`,
      `${bright("View:")}     ${color.underline(dim(`http://localhost:${port}`))}`,
      "",
      dim("The agent is now researching autonomously."),
      dim("Open the observatory to monitor progress."),
      "",
      dim("Press ") + bright("Ctrl+C") + dim(" to stop."),
    ].join("\n"),
    "Running"
  );

  // Wait for web server ready, then open browser
  const waitForServer = async () => {
    for (let i = 0; i < 20; i++) {
      try {
        await fetch(`http://localhost:${port}`);
        return true;
      } catch {
        await sleep(500);
      }
    }
    return false;
  };

  const ready = await waitForServer();
  if (ready && options.open !== false) {
    const openCmd =
      process.platform === "darwin" ? "open" :
      process.platform === "win32" ? "start" : "xdg-open";
    exec(`${openCmd} http://localhost:${port}`);
  }

  p.outro(green("Research in progress") + dim(" — observatory at localhost:" + port));

  // Keep alive
  process.on("SIGINT", () => {
    console.log();
    const s4 = p.spinner();
    // Can't use async spinner in signal handler, just print
    console.log(dim("  Shutting down..."));
    try { if (cookProc.pid) process.kill(-cookProc.pid, "SIGTERM"); } catch {}
    webServer.kill();
    try { require("fs").unlinkSync(pidFile); } catch {}
    console.log(green("  ✓") + " Stopped. Research state preserved.");
    console.log();
    process.exit(0);
  });

  process.on("SIGTERM", () => {
    try { if (cookProc.pid) process.kill(-cookProc.pid, "SIGTERM"); } catch {}
    webServer.kill();
    try { require("fs").unlinkSync(pidFile); } catch {}
    process.exit(0);
  });
}
