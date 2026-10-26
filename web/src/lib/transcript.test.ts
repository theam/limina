import { describe, expect, it } from "vitest";
import { buildTranscript, summarizeSteps } from "./transcript";
import type { Guidance, LiminaEvent } from "./types";

let sequence = 0;
function event(
  type: string,
  detail: Record<string, unknown> = {},
  extra: Partial<LiminaEvent> = {},
): LiminaEvent {
  sequence += 1;
  return {
    sequence,
    type,
    actor: "Limina",
    artifact_id: null,
    detail,
    created_at: new Date(Date.UTC(2026, 8, 29, 10, 0, sequence)).toISOString(),
    ...extra,
  };
}

describe("transcript", () => {
  it("groups a turn into messages, merged tool steps, knowledge writes and a checkpoint", () => {
    const events = [
      event(
        "guidance.received",
        { guidance_id: "g1", body: "Focus on p95", kind: "STEER" },
        { actor: "Ada" },
      ),
      event("runtime.turn_started", {
        run_id: "r1",
        summary: "Measure latency",
      }),
      event("runtime.codex", {
        run_id: "r1",
        kind: "tool",
        tool: "command",
        input: "pytest",
        item_id: "i1",
        phase: "started",
        state: "running",
      }),
      event("runtime.codex", {
        run_id: "r1",
        kind: "tool",
        tool: "command",
        input: "pytest",
        item_id: "i1",
        phase: "completed",
        state: "failed",
        exit_code: 1,
      }),
      event("runtime.codex", {
        run_id: "r1",
        kind: "knowledge",
        tool: "limina",
        item_id: "i2",
        phase: "started",
      }),
      event("hypothesis.created", { run_id: "r1" }, { artifact_id: "H001" }),
      event("runtime.codex", {
        run_id: "r1",
        kind: "message",
        text: "I proposed **H001**.",
      }),
      event("project.checkpoint", {
        status: "WAITING",
        next_step: "Run E001",
        blocker: "Need dataset access",
      }),
      event("runtime.turn_completed", {
        run_id: "r1",
        summary: "Proposed H001",
        status: "WAITING",
      }),
      event("guidance.incorporated", { guidance_id: "g1" }),
    ];
    const transcript = buildTranscript(events);
    expect(transcript.items.map((item) => item.type)).toEqual([
      "human",
      "turn",
      "activity",
      "artifact",
      "message",
      "checkpoint",
    ]);
    // The knowledge command folds into the artifact write it produced.
    const activity = transcript.items[2];
    expect(
      activity.type === "activity" &&
        activity.steps.map((step) => [step.state, step.exitCode]),
    ).toEqual([["failed", 1]]);
    const checkpoint = transcript.items[5];
    expect(checkpoint.type === "checkpoint" && checkpoint.blocker).toBe(
      "Need dataset access",
    );
    const human = transcript.items[0];
    expect(human.type === "human" && human.incorporatedAt).toBeTruthy();
    expect(transcript.activeRunId).toBeNull();
  });

  it("reports the active run and its latest step", () => {
    const transcript = buildTranscript([
      event("runtime.turn_started", { run_id: "r2", summary: "Next" }),
      event("runtime.claude-code", {
        run_id: "r2",
        kind: "tool",
        tool: "Read",
        input: "src/app.py",
        item_id: "t1",
        phase: "started",
      }),
    ]);
    expect(transcript.activeRunId).toBe("r2");
    expect(transcript.activeLabel).toBe("Reading project files");
  });

  it("keeps raw commands in execution history without exposing them in the primary working status", () => {
    const command = "python3 /private/project/scripts/check_membership.py --verbose";
    const transcript = buildTranscript([
      event("runtime.turn_started", { run_id: "live-run" }),
      event("runtime.codex", {
        run_id: "live-run", kind: "tool", tool: "command", input: command,
        item_id: "live-command", phase: "started", state: "running",
      }),
    ]);
    expect(transcript.activeLabel).not.toContain(command);
    expect(transcript.activeLabel).not.toContain("/private/project");
    const activity = transcript.items.find((item) => item.type === "activity");
    expect(activity?.type === "activity" && activity.steps[0].input).toBe(command);
  });

  it("degrades legacy provider rows and keeps optimistic messages until their event arrives", () => {
    const transcript = buildTranscript(
      [
        event("runtime.claude-code", {
          method: "assistant/message",
          summary: "Legacy text",
        }),
      ],
      [
        {
          localId: "l1",
          body: "hi",
          kind: "STEER",
          actor: "Ada",
          at: new Date().toISOString(),
          guidanceId: null,
          delivery: "SENDING",
        },
      ],
    );
    expect(transcript.items.map((item) => item.type)).toEqual([
      "message",
      "human",
    ]);
  });

  it("summarizes steps by kind", () => {
    const base = {
      key: "",
      input: null,
      summary: "",
      state: "ok" as const,
      exitCode: null,
      durationMs: null,
      files: [],
      at: "",
    };
    expect(
      summarizeSteps([
        { ...base, kind: "tool", tool: "Bash" },
        { ...base, kind: "tool", tool: "command" },
        { ...base, kind: "tool", tool: "Read" },
        { ...base, kind: "knowledge", tool: "limina" },
      ]),
    ).toBe("2 commands, 1 read, 1 knowledge update");
  });

  it("separates expected cancellation from a provider failure", () => {
    const transcript = buildTranscript([
      event("runtime.turn_interrupted", { summary: "Work paused", error: "Codex completed without the required project decision." }),
      event("runtime.turn_failed", { summary: "Turn failed", error: "Model access denied" }),
    ]);
    const notices = transcript.items.filter((item) => item.type === "notice");
    expect(notices).toMatchObject([
      { tone: "neutral", text: "Work paused", detail: null },
      { tone: "critical", text: "Turn failed", detail: "Model access denied" },
    ]);
  });

  it("settles steps left running when a turn ends and shows interrupts as notices", () => {
    const transcript = buildTranscript([
      event("runtime.turn_started", { run_id: "r3" }),
      event("runtime.codex", {
        run_id: "r3",
        kind: "tool",
        tool: "command",
        input: "sleep",
        item_id: "s1",
        phase: "started",
      }),
      event(
        "guidance.received",
        { guidance_id: "g9", kind: "INTERRUPT", body: "Pause" },
        { actor: "Ada" },
      ),
      event("runtime.turn_interrupted", {
        run_id: "r3",
        summary: "Interrupted",
      }),
    ]);
    const activity = transcript.items.find((item) => item.type === "activity");
    expect(activity?.type === "activity" && activity.steps[0].state).toBe(
      "stopped",
    );
    expect(
      transcript.items.some(
        (item) =>
          item.type === "notice" && item.text === "Ada interrupted the turn",
      ),
    ).toBe(true);
    expect(transcript.activeRunId).toBeNull();
  });

  it("hides an optimistic message once its live event arrives before the POST returns", () => {
    const sent = event(
      "guidance.received",
      { guidance_id: "g7", body: "Use the dev split", kind: "STEER" },
      { actor: "Ada" },
    );
    const transcript = buildTranscript(
      [sent],
      [
        {
          localId: "l2",
          body: "Use the dev split",
          kind: "STEER",
          actor: "Ada",
          at: sent.created_at,
          guidanceId: null,
          delivery: "SENDING",
        },
      ],
    );
    expect(
      transcript.items.filter((item) => item.type === "human"),
    ).toHaveLength(1);
  });
});

it("uses persisted acknowledgments when a checkpoint has no individual incorporation event", () => {
  const events = [
    event("guidance.received", {
      guidance_id: "approved",
      body: "Keep the guardrail",
      kind: "APPROVAL",
    }),
    event("project.checkpoint", { inbox_cursor: 4, messages_acknowledged: 1 }),
  ];
  const unknown = buildTranscript(events).items.find(
    (item) => item.type === "human",
  );
  expect(unknown?.type === "human" && unknown.incorporatedAt).toBeNull();
  const receipt = {
    id: "approved",
    acknowledged_at: "2026-10-23T10:00:00Z",
  } as Guidance;
  const confirmed = buildTranscript(events, [], [receipt]).items.find(
    (item) => item.type === "human",
  );
  expect(confirmed?.type === "human" && confirmed.incorporatedAt).toBe(
    receipt.acknowledged_at,
  );
  const unrelated = buildTranscript(
    events,
    [],
    [{ ...receipt, id: "other" }],
  ).items.find((item) => item.type === "human");
  expect(unrelated?.type === "human" && unrelated.incorporatedAt).toBeNull();
});
