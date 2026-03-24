import { execSync } from "child_process";
import { existsSync, readFileSync, writeFileSync, copyFileSync } from "fs";
import { join } from "path";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type CheckSeverity = "critical" | "warning" | "info";

export interface CheckResult {
  name: string;
  ok: boolean;
  severity: CheckSeverity;
  message: string;
  fix?: string;
  installable?: boolean;
}

export interface PreflightReport {
  checks: CheckResult[];
  ok: boolean;
  criticalFailures: number;
  warnings: number;
}

export type CommandExecutor = (cmd: string) => { ok: boolean; stdout: string };

// ---------------------------------------------------------------------------
// Shared helper
// ---------------------------------------------------------------------------

function defaultExec(cmd: string): { ok: boolean; stdout: string } {
  try {
    const stdout = execSync(cmd, { stdio: "pipe", timeout: 15_000 })
      .toString()
      .trim();
    return { ok: true, stdout };
  } catch {
    return { ok: false, stdout: "" };
  }
}

// ---------------------------------------------------------------------------
// Check functions
// ---------------------------------------------------------------------------

export function checkNodeVersion(
  exec: CommandExecutor = defaultExec,
): CheckResult {
  const result = exec("node --version");
  if (!result.ok) {
    return {
      name: "Node.js",
      ok: false,
      severity: "info",
      message: "not found",
      fix: "Install Node.js >= 18 from https://nodejs.org",
    };
  }
  const version = result.stdout.replace(/^v/, "");
  const major = parseInt(version.split(".")[0], 10);
  if (major < 18) {
    return {
      name: "Node.js",
      ok: false,
      severity: "critical",
      message: `v${version} (too old)`,
      fix: "Upgrade Node.js to >= 18 — https://nodejs.org",
    };
  }
  return { name: "Node.js", ok: true, severity: "info", message: `v${version}` };
}

export function checkNpm(
  exec: CommandExecutor = defaultExec,
): CheckResult {
  const result = exec("npm --version");
  if (!result.ok) {
    return {
      name: "npm",
      ok: false,
      severity: "info",
      message: "not found",
      fix: "npm is bundled with Node.js — reinstall Node",
    };
  }
  return { name: "npm", ok: true, severity: "info", message: `v${result.stdout}` };
}

export function checkGit(
  exec: CommandExecutor = defaultExec,
): CheckResult {
  const result = exec("git --version");
  if (!result.ok) {
    return {
      name: "git",
      ok: false,
      severity: "critical",
      message: "not found",
      fix: "Install git: https://git-scm.com",
      installable: true,
    };
  }
  const version = result.stdout.replace(/^git version\s*/, "").trim();
  return { name: "git", ok: true, severity: "critical", message: version };
}

export function checkAgentSdk(
  rootDir: string = join(__dirname, ".."),
): CheckResult {
  try {
    const pkgPath = join(rootDir, "node_modules", "@anthropic-ai", "claude-agent-sdk", "package.json");
    if (existsSync(pkgPath)) {
      const pkg = JSON.parse(readFileSync(pkgPath, "utf-8"));
      return { name: "Agent SDK", ok: true, severity: "critical", message: `v${pkg.version}` };
    }
    return {
      name: "Agent SDK",
      ok: false,
      severity: "critical",
      message: "not found",
      fix: "npm install @anthropic-ai/claude-agent-sdk",
      installable: true,
    };
  } catch {
    return {
      name: "Agent SDK",
      ok: false,
      severity: "critical",
      message: "not found",
      fix: "npm install @anthropic-ai/claude-agent-sdk",
      installable: true,
    };
  }
}

export function checkClaudeCli(
  exec: CommandExecutor = defaultExec,
): CheckResult {
  const result = exec("claude --version");
  if (!result.ok) {
    return {
      name: "Claude Code",
      ok: false,
      severity: "critical",
      message: "not found",
      fix: "npm install -g @anthropic-ai/claude-code",
      installable: true,
    };
  }
  return { name: "Claude Code", ok: true, severity: "critical", message: result.stdout };
}

export function checkClaudeAuth(
  exec: CommandExecutor = defaultExec,
): CheckResult {
  const result = exec("claude auth status");
  if (!result.ok) {
    return {
      name: "Claude auth",
      ok: false,
      severity: "critical",
      message: "could not check auth status",
      fix: "Run: claude auth login",
      installable: true,
    };
  }
  try {
    const status = JSON.parse(result.stdout);
    if (status.loggedIn) {
      const plan = status.subscriptionType ? ` (${status.subscriptionType})` : "";
      return {
        name: "Claude auth",
        ok: true,
        severity: "critical",
        message: `${status.email || "authenticated"}${plan}`,
      };
    }
    return {
      name: "Claude auth",
      ok: false,
      severity: "critical",
      message: "not logged in",
      fix: "Run: claude auth login",
      installable: true,
    };
  } catch {
    return {
      name: "Claude auth",
      ok: false,
      severity: "critical",
      message: "unexpected auth status response",
      fix: "Run: claude auth login",
      installable: true,
    };
  }
}

export function checkNodeModules(
  rootDir: string = join(__dirname, ".."),
): CheckResult {
  const nextBin = join(rootDir, "node_modules", ".bin", "next");
  if (existsSync(nextBin)) {
    return { name: "node_modules", ok: true, severity: "critical", message: "installed" };
  }
  return {
    name: "node_modules",
    ok: false,
    severity: "critical",
    message: "not installed",
    fix: `cd ${rootDir} && npm install`,
    installable: true,
  };
}

export function checkEnvFile(
  rootDir: string = join(__dirname, ".."),
): CheckResult {
  const envPath = join(rootDir, ".env.local");
  if (!existsSync(envPath)) {
    return {
      name: ".env.local",
      ok: false,
      severity: "warning",
      message: "not found",
      fix: "cp .env.example .env.local && edit .env.local",
      installable: true,
    };
  }
  const content = readFileSync(envPath, "utf-8");
  if (
    content.includes("your-secret-api-key-here") ||
    !/MISSION_API_KEY=.+/.test(content)
  ) {
    return {
      name: ".env.local",
      ok: false,
      severity: "warning",
      message: "MISSION_API_KEY is placeholder or empty",
      fix: "Edit .env.local and set a real MISSION_API_KEY",
      installable: true,
    };
  }
  return { name: ".env.local", ok: true, severity: "warning", message: "configured" };
}

// ---------------------------------------------------------------------------
// Install functions
// ---------------------------------------------------------------------------

export function installGit(): { ok: boolean; error?: string } {
  const platform = process.platform;
  const sudo = process.getuid?.() === 0 ? "" : "sudo ";
  try {
    if (platform === "darwin") {
      const hasBrew = defaultExec("which brew");
      if (hasBrew.ok) {
        execSync("brew install git", { stdio: "pipe", timeout: 120_000 });
        return { ok: true };
      }
      execSync("xcode-select --install", { stdio: "pipe", timeout: 120_000 });
      return { ok: true };
    } else if (platform === "linux") {
      if (defaultExec("which apt-get").ok) {
        execSync(`${sudo}apt-get update -qq && ${sudo}apt-get install -y -qq git`, {
          stdio: "pipe",
          timeout: 120_000,
        });
        return { ok: true };
      }
      if (defaultExec("which pacman").ok) {
        execSync(`${sudo}pacman -Sy --noconfirm git`, { stdio: "pipe", timeout: 120_000 });
        return { ok: true };
      }
      if (defaultExec("which dnf").ok) {
        execSync(`${sudo}dnf install -y -q git`, { stdio: "pipe", timeout: 120_000 });
        return { ok: true };
      }
    }
    return { ok: false, error: "Unsupported platform — install git manually" };
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : String(e);
    return { ok: false, error: msg };
  }
}

export function installAgentSdk(
  rootDir: string = join(__dirname, ".."),
): { ok: boolean; error?: string } {
  try {
    execSync("npm install @anthropic-ai/claude-agent-sdk", {
      cwd: rootDir,
      stdio: "pipe",
      timeout: 120_000,
    });
    return { ok: true };
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : String(e);
    return { ok: false, error: msg };
  }
}

export function installClaudeCli(): { ok: boolean; error?: string } {
  try {
    execSync("npm install -g @anthropic-ai/claude-code", {
      stdio: "pipe",
      timeout: 120_000,
    });
    return { ok: true };
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : String(e);
    return { ok: false, error: msg };
  }
}

// Claude auth is handled interactively by the caller — this is a
// re-check function, not an automated installer. The caller should
// prompt the user to run `claude auth login` in another terminal,
// then call this to verify.
export function installClaudeAuth(): { ok: boolean; error?: string } {
  const check = checkClaudeAuth();
  if (check.ok) {
    return { ok: true };
  }
  return { ok: false, error: "Not yet authenticated" };
}

export function installNodeModules(
  rootDir: string = join(__dirname, ".."),
): { ok: boolean; error?: string } {
  try {
    execSync("npm install", { cwd: rootDir, stdio: "pipe", timeout: 180_000 });
    return { ok: true };
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : String(e);
    return { ok: false, error: msg };
  }
}

export function installEnvFile(
  rootDir: string = join(__dirname, ".."),
): { ok: boolean; error?: string } {
  try {
    const examplePath = join(rootDir, ".env.example");
    const envPath = join(rootDir, ".env.local");
    if (existsSync(examplePath)) {
      copyFileSync(examplePath, envPath);
      // Replace placeholder with random key
      let content = readFileSync(envPath, "utf-8");
      const randomKey = `limina_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 10)}`;
      content = content.replace("your-secret-api-key-here", randomKey);
      writeFileSync(envPath, content);
    } else {
      writeFileSync(envPath, `MISSION_API_KEY=limina_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 10)}\n`);
    }
    return { ok: true };
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : String(e);
    return { ok: false, error: msg };
  }
}

// Map check names to install functions
export const installers: Record<string, () => { ok: boolean; error?: string }> = {
  git: installGit,
  "Agent SDK": installAgentSdk,
  "Claude Code": installClaudeCli,
  "Claude auth": installClaudeAuth,
  node_modules: installNodeModules,
  ".env.local": installEnvFile,
};

// ---------------------------------------------------------------------------
// Composite runners
// ---------------------------------------------------------------------------

function buildReport(checks: CheckResult[]): PreflightReport {
  const criticalFailures = checks.filter(
    (c) => c.severity === "critical" && !c.ok,
  ).length;
  const warnings = checks.filter(
    (c) => c.severity === "warning" && !c.ok,
  ).length;
  return {
    checks,
    ok: criticalFailures === 0,
    criticalFailures,
    warnings,
  };
}

export function runAllChecks(
  exec?: CommandExecutor,
  rootDir?: string,
): PreflightReport {
  const checks = [
    checkNodeVersion(exec),
    checkNpm(exec),
    checkGit(exec),
    checkAgentSdk(rootDir),
    checkClaudeCli(exec),
    checkClaudeAuth(exec),
    checkNodeModules(rootDir),
    checkEnvFile(rootDir),
  ];
  return buildReport(checks);
}

export function runInitPreflight(exec?: CommandExecutor): PreflightReport {
  const checks = [
    checkNodeVersion(exec),
    checkNpm(exec),
    checkGit(exec),
    checkClaudeCli(exec),
    checkClaudeAuth(exec),
  ];
  return buildReport(checks);
}

export function runStartPreflight(
  exec?: CommandExecutor,
  rootDir?: string,
): PreflightReport {
  const checks = [
    checkAgentSdk(rootDir),
    checkClaudeCli(exec),
    checkClaudeAuth(exec),
    checkNodeModules(rootDir),
    checkEnvFile(rootDir),
  ];
  return buildReport(checks);
}
