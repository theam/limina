import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { mkdirSync, writeFileSync, rmSync, existsSync } from "fs";
import { join } from "path";
import { tmpdir } from "os";
import {
  checkNodeVersion,
  checkNpm,
  checkGit,
  checkAgentSdk,
  checkClaudeCli,
  checkClaudeAuth,
  checkNodeModules,
  checkEnvFile,
  runAllChecks,
  runInitPreflight,
  runStartPreflight,
  type CommandExecutor,
} from "../../cli/preflight";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const ok = (stdout: string): CommandExecutor => () => ({ ok: true, stdout });
const fail = (): CommandExecutor => () => ({ ok: false, stdout: "" });

let tmpDir: string;

beforeEach(() => {
  tmpDir = join(tmpdir(), `limina-test-${Date.now()}`);
  mkdirSync(tmpDir, { recursive: true });
});

afterEach(() => {
  if (existsSync(tmpDir)) {
    rmSync(tmpDir, { recursive: true, force: true });
  }
});

// ---------------------------------------------------------------------------
// checkNodeVersion
// ---------------------------------------------------------------------------

describe("checkNodeVersion", () => {
  it("passes for Node v20", () => {
    const result = checkNodeVersion(ok("v20.19.3"));
    expect(result.ok).toBe(true);
    expect(result.message).toBe("v20.19.3");
  });

  it("passes at the v18 boundary", () => {
    const result = checkNodeVersion(ok("v18.0.0"));
    expect(result.ok).toBe(true);
    expect(result.message).toBe("v18.0.0");
  });

  it("fails for Node v16", () => {
    const result = checkNodeVersion(ok("v16.14.0"));
    expect(result.ok).toBe(false);
    expect(result.message).toContain("too old");
    expect(result.fix).toContain("18");
  });

  it("fails when node is not found", () => {
    const result = checkNodeVersion(fail());
    expect(result.ok).toBe(false);
    expect(result.message).toBe("not found");
  });
});

// ---------------------------------------------------------------------------
// checkNpm
// ---------------------------------------------------------------------------

describe("checkNpm", () => {
  it("passes when npm is found", () => {
    const result = checkNpm(ok("10.9.2"));
    expect(result.ok).toBe(true);
    expect(result.message).toBe("v10.9.2");
  });

  it("fails when npm is not found", () => {
    const result = checkNpm(fail());
    expect(result.ok).toBe(false);
    expect(result.fix).toContain("Node");
  });
});

// ---------------------------------------------------------------------------
// checkGit
// ---------------------------------------------------------------------------

describe("checkGit", () => {
  it("passes when git is found", () => {
    const result = checkGit(ok("git version 2.47.2"));
    expect(result.ok).toBe(true);
    expect(result.message).toBe("2.47.2");
  });

  it("fails with installable flag when git is not found", () => {
    const result = checkGit(fail());
    expect(result.ok).toBe(false);
    expect(result.installable).toBe(true);
    expect(result.fix).toContain("git");
  });
});

// ---------------------------------------------------------------------------
// checkAgentSdk
// ---------------------------------------------------------------------------

describe("checkAgentSdk", () => {
  it("passes when SDK package exists in node_modules", () => {
    const sdkDir = join(tmpDir, "node_modules", "@anthropic-ai", "claude-agent-sdk");
    mkdirSync(sdkDir, { recursive: true });
    writeFileSync(join(sdkDir, "package.json"), JSON.stringify({ version: "0.2.81" }));
    const result = checkAgentSdk(tmpDir);
    expect(result.ok).toBe(true);
    expect(result.message).toBe("v0.2.81");
  });

  it("fails with installable flag when SDK is not found", () => {
    const result = checkAgentSdk(tmpDir);
    expect(result.ok).toBe(false);
    expect(result.installable).toBe(true);
    expect(result.fix).toContain("@anthropic-ai/claude-agent-sdk");
  });
});

// ---------------------------------------------------------------------------
// checkClaudeCli
// ---------------------------------------------------------------------------

describe("checkClaudeCli", () => {
  it("passes and shows version string", () => {
    const result = checkClaudeCli(ok("2.1.81 (Claude Code)"));
    expect(result.ok).toBe(true);
    expect(result.message).toBe("2.1.81 (Claude Code)");
  });

  it("fails with installable flag when not found", () => {
    const result = checkClaudeCli(fail());
    expect(result.ok).toBe(false);
    expect(result.installable).toBe(true);
    expect(result.fix).toContain("@anthropic-ai/claude-code");
  });
});

// ---------------------------------------------------------------------------
// checkClaudeAuth
// ---------------------------------------------------------------------------

describe("checkClaudeAuth", () => {
  it("passes when logged in with email and plan", () => {
    const json = JSON.stringify({
      loggedIn: true,
      email: "user@example.com",
      subscriptionType: "max",
    });
    const result = checkClaudeAuth(ok(json));
    expect(result.ok).toBe(true);
    expect(result.message).toContain("user@example.com");
    expect(result.message).toContain("max");
  });

  it("passes when logged in without subscription type", () => {
    const json = JSON.stringify({ loggedIn: true, email: "user@example.com" });
    const result = checkClaudeAuth(ok(json));
    expect(result.ok).toBe(true);
    expect(result.message).toBe("user@example.com");
  });

  it("fails when not logged in", () => {
    const json = JSON.stringify({ loggedIn: false });
    const result = checkClaudeAuth(ok(json));
    expect(result.ok).toBe(false);
    expect(result.message).toBe("not logged in");
    expect(result.installable).toBe(true);
    expect(result.fix).toContain("claude auth login");
  });

  it("fails on invalid JSON", () => {
    const result = checkClaudeAuth(ok("not json"));
    expect(result.ok).toBe(false);
    expect(result.message).toBe("unexpected auth status response");
    expect(result.installable).toBe(true);
  });

  it("fails when command is not available", () => {
    const result = checkClaudeAuth(fail());
    expect(result.ok).toBe(false);
    expect(result.message).toBe("could not check auth status");
  });
});

// ---------------------------------------------------------------------------
// checkNodeModules
// ---------------------------------------------------------------------------

describe("checkNodeModules", () => {
  it("passes when node_modules/.bin/next exists", () => {
    const binDir = join(tmpDir, "node_modules", ".bin");
    mkdirSync(binDir, { recursive: true });
    writeFileSync(join(binDir, "next"), "");
    const result = checkNodeModules(tmpDir);
    expect(result.ok).toBe(true);
  });

  it("fails with installable flag when missing", () => {
    const result = checkNodeModules(tmpDir);
    expect(result.ok).toBe(false);
    expect(result.installable).toBe(true);
    expect(result.fix).toContain("npm install");
  });
});

// ---------------------------------------------------------------------------
// checkEnvFile
// ---------------------------------------------------------------------------

describe("checkEnvFile", () => {
  it("passes with a real API key", () => {
    writeFileSync(join(tmpDir, ".env.local"), "MISSION_API_KEY=my-real-key\n");
    const result = checkEnvFile(tmpDir);
    expect(result.ok).toBe(true);
    expect(result.message).toBe("configured");
  });

  it("fails when file is missing", () => {
    const result = checkEnvFile(tmpDir);
    expect(result.ok).toBe(false);
    expect(result.severity).toBe("warning");
    expect(result.installable).toBe(true);
  });

  it("fails when MISSION_API_KEY is the placeholder", () => {
    writeFileSync(
      join(tmpDir, ".env.local"),
      "MISSION_API_KEY=your-secret-api-key-here\n",
    );
    const result = checkEnvFile(tmpDir);
    expect(result.ok).toBe(false);
    expect(result.message).toContain("placeholder");
  });

  it("fails when MISSION_API_KEY is empty", () => {
    writeFileSync(join(tmpDir, ".env.local"), "MISSION_API_KEY=\n");
    const result = checkEnvFile(tmpDir);
    expect(result.ok).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Composite runners
// ---------------------------------------------------------------------------

describe("runAllChecks", () => {
  it("returns 8 checks", () => {
    const report = runAllChecks(ok("v20.0.0"), tmpDir);
    expect(report.checks).toHaveLength(8);
  });

  it("reports ok when all pass", () => {
    // Set up filesystem checks to pass
    const binDir = join(tmpDir, "node_modules", ".bin");
    mkdirSync(binDir, { recursive: true });
    writeFileSync(join(binDir, "next"), "");
    writeFileSync(join(tmpDir, ".env.local"), "MISSION_API_KEY=real-key\n");

    // Also set up Agent SDK
    const sdkDir = join(tmpDir, "node_modules", "@anthropic-ai", "claude-agent-sdk");
    mkdirSync(sdkDir, { recursive: true });
    writeFileSync(join(sdkDir, "package.json"), JSON.stringify({ version: "0.2.81" }));

    const allPass: CommandExecutor = (cmd: string) => {
      if (cmd === "node --version") return { ok: true, stdout: "v20.0.0" };
      if (cmd === "npm --version") return { ok: true, stdout: "10.0.0" };
      if (cmd === "git --version") return { ok: true, stdout: "git version 2.47.0" };
      if (cmd === "claude --version") return { ok: true, stdout: "2.1.81 (Claude Code)" };
      if (cmd === "claude auth status")
        return { ok: true, stdout: JSON.stringify({ loggedIn: true, email: "a@b.com" }) };
      return { ok: true, stdout: "" };
    };

    const report = runAllChecks(allPass, tmpDir);
    expect(report.ok).toBe(true);
    expect(report.criticalFailures).toBe(0);
    expect(report.warnings).toBe(0);
  });

  it("reports not ok when a critical check fails", () => {
    const report = runAllChecks(fail(), tmpDir);
    expect(report.ok).toBe(false);
    expect(report.criticalFailures).toBeGreaterThan(0);
  });
});

describe("runInitPreflight", () => {
  it("returns 5 checks (Node, npm, git, claude, claude auth)", () => {
    const report = runInitPreflight(ok("v20.0.0"));
    expect(report.checks).toHaveLength(5);
    const names = report.checks.map((c) => c.name);
    expect(names).toContain("Node.js");
    expect(names).toContain("npm");
    expect(names).toContain("git");
    expect(names).toContain("Claude Code");
    expect(names).toContain("Claude auth");
  });

  it("does not include node_modules or .env.local", () => {
    const report = runInitPreflight(ok("v20.0.0"));
    const names = report.checks.map((c) => c.name);
    expect(names).not.toContain("node_modules");
    expect(names).not.toContain(".env.local");
  });
});

describe("runStartPreflight", () => {
  it("returns 5 checks (Agent SDK, claude, claude auth, node_modules, env)", () => {
    const report = runStartPreflight(ok(""), tmpDir);
    expect(report.checks).toHaveLength(5);
    const names = report.checks.map((c) => c.name);
    expect(names).toContain("Agent SDK");
    expect(names).toContain("Claude Code");
    expect(names).toContain("Claude auth");
    expect(names).toContain("node_modules");
    expect(names).toContain(".env.local");
  });

  it("reports ok when only warning checks fail", () => {
    // node_modules passes, .env.local fails (warning)
    const binDir = join(tmpDir, "node_modules", ".bin");
    mkdirSync(binDir, { recursive: true });
    writeFileSync(join(binDir, "next"), "");
    // Agent SDK passes
    const sdkDir = join(tmpDir, "node_modules", "@anthropic-ai", "claude-agent-sdk");
    mkdirSync(sdkDir, { recursive: true });
    writeFileSync(join(sdkDir, "package.json"), JSON.stringify({ version: "0.2.81" }));
    // .env.local missing → warning

    const allCriticalPass: CommandExecutor = (cmd: string) => {
      if (cmd === "claude --version") return { ok: true, stdout: "2.1.81" };
      if (cmd === "claude auth status")
        return { ok: true, stdout: JSON.stringify({ loggedIn: true, email: "a@b.com" }) };
      return { ok: true, stdout: "" };
    };

    const report = runStartPreflight(allCriticalPass, tmpDir);
    expect(report.ok).toBe(true);
    expect(report.warnings).toBe(1);
  });
});
