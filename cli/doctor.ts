import * as p from "@clack/prompts";
import color from "chalk";
import {
  runAllChecks,
  installers,
  checkClaudeCli,
  checkClaudeAuth,
  type CheckResult,
} from "./preflight";

const blue = color.hex("#0f62fe");
const green = color.hex("#198038");
const red = color.hex("#da1e28");
const yellow = color.hex("#f1c21b");
const dim = color.dim;

function renderCheck(check: CheckResult) {
  if (check.ok) {
    console.log(green("  ✓  ") + check.name + "  " + dim(check.message));
  } else if (check.severity === "warning") {
    console.log(yellow("  ⚠  ") + check.name + "  " + dim(check.message));
    if (check.fix) console.log(dim("     → ") + check.fix);
  } else {
    console.log(red("  ✗  ") + check.name + "  " + dim(check.message));
    if (check.fix) console.log(dim("     → ") + check.fix);
  }
}

export async function doctor() {
  console.log();
  p.intro(blue("Limina Doctor"));

  // Phase 1: Check everything
  const s = p.spinner();
  s.start("Checking prerequisites");
  const report = runAllChecks();
  s.stop(
    report.ok && report.warnings === 0
      ? green("✓") + " All checks passed"
      : report.ok
        ? green("✓") + " Checks complete " + yellow(`(${report.warnings} warning${report.warnings > 1 ? "s" : ""})`)
        : red("✗") + ` ${report.criticalFailures} issue${report.criticalFailures > 1 ? "s" : ""} found`,
  );

  // Phase 2: Display results grouped
  const systemChecks = report.checks.filter((c) =>
    ["Node.js", "npm", "git"].includes(c.name),
  );
  const agentChecks = report.checks.filter((c) =>
    ["Agent SDK", "Claude Code", "Claude auth"].includes(c.name),
  );
  const projectChecks = report.checks.filter((c) =>
    ["node_modules", ".env.local"].includes(c.name),
  );

  if (systemChecks.length) {
    console.log();
    console.log(dim("  System"));
    for (const check of systemChecks) renderCheck(check);
  }

  if (agentChecks.length) {
    console.log();
    console.log(dim("  Agent Stack"));
    for (const check of agentChecks) renderCheck(check);
  }

  if (projectChecks.length) {
    console.log();
    console.log(dim("  Project"));
    for (const check of projectChecks) renderCheck(check);
  }

  // Phase 3: Auto-install missing prerequisites
  const installable = report.checks.filter((c) => !c.ok && c.installable);
  if (installable.length > 0) {
    console.log();
    const shouldInstall = await p.confirm({
      message: `Install ${installable.length} missing prerequisite${installable.length > 1 ? "s" : ""}?`,
      initialValue: true,
    });

    if (shouldInstall && !p.isCancel(shouldInstall)) {
      console.log();
      for (const check of installable) {
        // Special case: Claude auth is interactive
        if (check.name === "Claude auth") {
          // First make sure Claude CLI is installed (it may have just been installed above)
          const claudeCheck = checkClaudeCli();
          if (!claudeCheck.ok) {
            console.log(red("  ✗  ") + "Cannot authenticate — Claude Code not installed");
            continue;
          }
          console.log();
          console.log(blue("  ◇  ") + "Claude Code needs authentication");
          console.log();
          console.log(dim("     Run this in another terminal:"));
          console.log();
          console.log("       " + color.bold("claude auth login"));
          console.log();
          await p.text({
            message: "Press Enter when done",
            defaultValue: "",
            placeholder: "",
          });
          const recheck = checkClaudeAuth();
          if (recheck.ok) {
            console.log(green("  ✓  ") + "Authenticated as " + dim(recheck.message));
          } else {
            console.log(red("  ✗  ") + "Still not authenticated — run " + color.bold("claude auth login") + " and try again");
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
          if (result.error) {
            // Show just the last meaningful line of error
            const lines = result.error.split("\n").filter((l) => l.trim());
            const lastLine = lines[lines.length - 1] || result.error;
            console.log(dim("     " + lastLine.slice(0, 120)));
          }
          if (check.fix) {
            console.log(dim("     Manual fix: ") + check.fix);
          }
        }
      }
    }
  }

  // Phase 4: Final status
  console.log();
  const final = runAllChecks();
  if (final.ok && final.warnings === 0) {
    p.outro(green("All prerequisites installed and verified"));
  } else if (final.ok) {
    p.outro(
      green("All critical checks passed") +
        dim(` (${final.warnings} warning${final.warnings > 1 ? "s" : ""})`),
    );
  } else {
    const remaining = final.checks.filter(
      (c) => !c.ok && c.severity === "critical",
    );
    console.log();
    for (const check of remaining) renderCheck(check);
    console.log();
    p.outro(
      red(`${final.criticalFailures} critical issue${final.criticalFailures > 1 ? "s" : ""} remaining`) +
        dim(" — fix manually and run ") +
        color.bold("limina doctor") +
        dim(" again"),
    );
  }
}
