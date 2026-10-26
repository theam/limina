/**
 * Turn Limina's durable event stream into a readable conversation.
 *
 * Provider events (runtime.codex / runtime.claude-code) carry a normalized `detail.kind`;
 * older rows without it degrade to status lines. Tool rows with the same `item_id` merge
 * their started/completed halves, consecutive tool rows collapse into one activity group,
 * and human guidance is matched to its incorporation acknowledgement.
 */

import type { EventDetail, Guidance, GuidanceKind, LiminaEvent } from "./types";
import type { Tone } from "./format";

export interface Step {
  key: string;
  kind: "tool" | "knowledge" | "status";
  tool: string;
  input: string | null;
  summary: string;
  state: "running" | "ok" | "failed" | "declined" | "stopped" | null;
  exitCode: number | null;
  durationMs: number | null;
  files: { path: string; kind: string }[];
  at: string;
}

export type TranscriptItem =
  | {
      type: "turn";
      key: string;
      at: string;
      runId: string | null;
      focus: string;
    }
  | {
      type: "message";
      key: string;
      at: string;
      runId: string | null;
      text: string;
    }
  | {
      type: "plan";
      key: string;
      at: string;
      runId: string | null;
      text: string;
    }
  | {
      type: "activity";
      key: string;
      at: string;
      runId: string | null;
      steps: Step[];
    }
  | {
      type: "human";
      key: string;
      at: string;
      guidanceId: string | null;
      actor: string;
      body: string;
      kind: GuidanceKind;
      incorporatedAt: string | null;
      delivery?: "LIVE" | "QUEUED" | "SENDING" | "FAILED";
    }
  | {
      type: "artifact";
      key: string;
      at: string;
      runId: string | null;
      artifactId: string;
      action: string;
    }
  | {
      type: "checkpoint";
      key: string;
      at: string;
      runId: string | null;
      summary: string;
      status: string;
      objective: string | null;
      nextStep: string | null;
      blocker: string | null;
    }
  | {
      type: "notice";
      key: string;
      at: string;
      tone: Tone;
      text: string;
      detail: string | null;
    };

export interface PendingMessage {
  localId: string;
  body: string;
  kind: GuidanceKind;
  actor: string;
  at: string;
  guidanceId: string | null;
  delivery: "SENDING" | "LIVE" | "QUEUED" | "FAILED";
}

export interface Transcript {
  items: TranscriptItem[];
  /** The run currently producing work, if its turn has started and not finished. */
  activeRunId: string | null;
  /** The latest step or message of the active run, for the "working" indicator. */
  activeLabel: string | null;
  activeSince: string | null;
}

const ARTIFACT_ACTIONS: Record<string, string> = {
  "hypothesis.created": "Proposed",
  "hypothesis.decided": "Decided",
  "experiment.created": "Designed",
  "experiment.observed": "Recorded an observation on",
  "experiment.completed": "Completed",
  "finding.published": "Published",
};

const LIFECYCLE: Record<string, string> = {
  "project.created": "created the project",
  "project.started": "started the project",
  "project.paused": "paused the project",
  "project.resumed": "resumed the project",
  "project.stopped": "stopped the project",
  "project.archived": "archived the project",
};

function text(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function stepFrom(event: LiminaEvent, detail: EventDetail): Step {
  const kind =
    detail.kind === "knowledge"
      ? "knowledge"
      : detail.kind === "status"
        ? "status"
        : "tool";
  const itemType = text(detail.item_type);
  return {
    key: detail.item_id ? `item:${detail.item_id}` : `event:${event.sequence}`,
    kind,
    tool:
      detail.tool ||
      (kind === "status"
        ? "status"
        : itemType.replace(/ThreadItem$/, "") || "tool"),
    input: detail.input ?? null,
    summary: detail.text || detail.summary || "",
    state:
      detail.state ??
      (kind === "status"
        ? null
        : detail.phase === "completed"
          ? "ok"
          : "running"),
    exitCode: detail.exit_code ?? null,
    durationMs: detail.duration_ms ?? null,
    files: detail.files ?? [],
    at: event.created_at,
  };
}

/** Classify legacy provider rows that predate the normalized `kind` field. */
function legacyKind(detail: EventDetail): EventDetail["kind"] {
  const method = text(detail.method);
  const itemType = text(detail.item_type);
  if (method === "assistant/message" || itemType.startsWith("AgentMessage"))
    return "message";
  if (
    method === "tool/use" ||
    /Command|Mcp|FileChange|WebSearch|ToolCall/.test(itemType)
  )
    return "tool";
  return "status";
}

export function buildTranscript(
  events: LiminaEvent[],
  pending: PendingMessage[] = [],
  receipts: Guidance[] = [],
): Transcript {
  const items: TranscriptItem[] = [];
  const steps = new Map<string, Step>();
  const humans = new Map<string, Extract<TranscriptItem, { type: "human" }>>();
  const openRuns = new Map<string, { at: string; label: string | null }>();
  let lastCheckpoint: Extract<TranscriptItem, { type: "checkpoint" }> | null =
    null;
  // The supervisor persists the checkpoint just before announcing turn completion.
  let recentCheckpoint: EventDetail | null = null;

  const settleRun = (runId: string | null) => {
    if (!runId) return;
    for (const item of items) {
      if (item.type !== "activity" || item.runId !== runId) continue;
      for (const step of item.steps)
        if (step.state === "running") step.state = "stopped";
    }
  };

  const pushStep = (
    event: LiminaEvent,
    detail: EventDetail,
    runId: string | null,
  ) => {
    const step = stepFrom(event, detail);
    const existing = steps.get(step.key);
    if (existing) {
      existing.state = step.state ?? existing.state;
      existing.exitCode = step.exitCode ?? existing.exitCode;
      existing.durationMs = step.durationMs ?? existing.durationMs;
      if (step.files.length) existing.files = step.files;
      if (step.input && !existing.input) existing.input = step.input;
      return existing;
    }
    steps.set(step.key, step);
    const last = items[items.length - 1];
    if (last?.type === "activity" && last.runId === runId) {
      last.steps.push(step);
    } else {
      items.push({
        type: "activity",
        key: `activity:${event.sequence}`,
        at: event.created_at,
        runId,
        steps: [step],
      });
    }
    return step;
  };

  for (const event of events) {
    const detail = event.detail ?? {};
    const runId = detail.run_id ?? null;
    const type = event.type;

    if (type === "runtime.codex" || type === "runtime.claude-code") {
      const kind = detail.kind ?? legacyKind(detail);
      if (kind === "message") {
        const body = detail.text || detail.summary || "";
        if (!body.trim()) continue;
        items.push({
          type: "message",
          key: `message:${event.sequence}`,
          at: event.created_at,
          runId,
          text: body,
        });
        if (runId && openRuns.has(runId))
          openRuns.set(runId, {
            at: openRuns.get(runId)!.at,
            label: "Writing a response",
          });
      } else if (kind === "plan") {
        items.push({
          type: "plan",
          key: `plan:${event.sequence}`,
          at: event.created_at,
          runId,
          text: detail.text || detail.summary || "",
        });
      } else {
        const step = pushStep(event, { ...detail, kind }, runId);
        if (runId && openRuns.has(runId)) {
          const tool = step.tool.toLowerCase();
          const label = step.kind === "knowledge"
            ? "Updating project knowledge"
            : /^(read|glob|grep|ls)$/.test(tool)
              ? "Reading project files"
              : /^(edit|write|multiedit|notebookedit)$/.test(tool)
                ? "Updating project files"
                : /search|fetch/.test(tool)
                  ? "Consulting sources"
                  : "Working on the current checkpoint";
          openRuns.set(runId, { at: openRuns.get(runId)!.at, label });
        }
      }
      continue;
    }

    if (type === "runtime.turn_started") {
      recentCheckpoint = null;
      items.push({
        type: "turn",
        key: `turn:${event.sequence}`,
        at: event.created_at,
        runId,
        focus: detail.summary ?? "",
      });
      if (runId) openRuns.set(runId, { at: event.created_at, label: null });
      continue;
    }

    if (type === "runtime.turn_completed") {
      if (runId) openRuns.delete(runId);
      settleRun(runId);
      lastCheckpoint = {
        type: "checkpoint",
        key: `checkpoint:${event.sequence}`,
        at: event.created_at,
        runId,
        summary: detail.summary ?? "",
        status: detail.status ?? "",
        objective: recentCheckpoint?.current_objective ?? null,
        nextStep: recentCheckpoint?.next_step ?? null,
        blocker: recentCheckpoint?.blocker ?? null,
      };
      recentCheckpoint = null;
      items.push(lastCheckpoint);
      continue;
    }

    if (type === "project.checkpoint") {
      recentCheckpoint = detail;
      continue;
    }

    if (type === "runtime.turn_failed" || type === "runtime.turn_interrupted") {
      if (runId) openRuns.delete(runId);
      settleRun(runId);
      const failed = type === "runtime.turn_failed";
      items.push({
        type: "notice",
        key: `notice:${event.sequence}`,
        at: event.created_at,
        tone: failed ? "critical" : "neutral",
        text:
          detail.summary ??
          (failed ? "The turn failed." : "The turn was interrupted."),
        detail: failed ? detail.error ?? null : null,
      });
      continue;
    }

    if (
      type === "runtime.retry_scheduled" ||
      type === "runtime.failed" ||
      type === "runtime.lease_lost"
    ) {
      items.push({
        type: "notice",
        key: `notice:${event.sequence}`,
        at: event.created_at,
        tone: type === "runtime.failed" ? "critical" : "warning",
        text: detail.summary ?? type,
        detail: detail.reason ?? detail.error ?? null,
      });
      continue;
    }

    if (type === "guidance.received" && detail.origin === "resource") {
      items.push({
        type: "notice",
        key: `notice:${event.sequence}`,
        at: event.created_at,
        tone: "neutral",
        text: `${event.actor} · ${detail.body ?? "Project resources changed"}`,
        detail: null,
      });
      continue;
    }

    if (type === "guidance.received" && String(detail.kind) === "INTERRUPT") {
      items.push({
        type: "notice",
        key: `notice:${event.sequence}`,
        at: event.created_at,
        tone: "warning",
        text: `${event.actor} interrupted the turn`,
        detail: null,
      });
      continue;
    }

    if (type === "guidance.received") {
      const guidanceId = detail.guidance_id ?? null;
      const item: Extract<TranscriptItem, { type: "human" }> = {
        type: "human",
        key: `human:${guidanceId ?? event.sequence}`,
        at: event.created_at,
        guidanceId,
        actor: event.actor,
        body: detail.body ?? "",
        kind: (detail.kind as unknown as GuidanceKind) ?? "STEER",
        incorporatedAt: null,
      };
      items.push(item);
      if (guidanceId) humans.set(guidanceId, item);
      continue;
    }

    if (type === "guidance.incorporated") {
      const target = detail.guidance_id
        ? humans.get(detail.guidance_id)
        : undefined;
      if (target) target.incorporatedAt = event.created_at;
      continue;
    }

    if (ARTIFACT_ACTIONS[type] && event.artifact_id) {
      // The knowledge command that produced this write is redundant next to the write itself.
      const last = items[items.length - 1];
      if (last?.type === "activity") {
        while (
          last.steps.length &&
          last.steps[last.steps.length - 1].kind === "knowledge"
        )
          last.steps.pop();
        if (!last.steps.length) items.pop();
      }
      items.push({
        type: "artifact",
        key: `artifact:${event.sequence}`,
        at: event.created_at,
        runId,
        artifactId: event.artifact_id,
        action: ARTIFACT_ACTIONS[type],
      });
      continue;
    }

    if (LIFECYCLE[type]) {
      items.push({
        type: "notice",
        key: `notice:${event.sequence}`,
        at: event.created_at,
        tone: "neutral",
        text: `${event.actor} ${LIFECYCLE[type]}`,
        detail: null,
      });
    }
  }

  // Optimistic messages stay visible until their durable event arrives.
  const durable = items.filter(
    (item): item is Extract<TranscriptItem, { type: "human" }> =>
      item.type === "human",
  );
  for (const message of pending) {
    if (message.guidanceId && humans.has(message.guidanceId)) continue;
    // The live event can arrive before the POST returns the id; match it by content and time.
    const sent = new Date(message.at).getTime();
    if (
      !message.guidanceId &&
      durable.some(
        (item) =>
          item.body === message.body &&
          item.kind === message.kind &&
          new Date(item.at).getTime() >= sent - 60_000,
      )
    )
      continue;
    items.push({
      type: "human",
      key: `pending:${message.localId}`,
      at: message.at,
      guidanceId: message.guidanceId,
      actor: message.actor,
      body: message.body,
      kind: message.kind,
      incorporatedAt: null,
      delivery: message.delivery,
    });
  }

  // Coordinator checkpoints acknowledge inbox rows transactionally without necessarily emitting
  // an individual guidance.incorporated event. Use the actual receipt, never the cursor as a proxy.
  const acknowledged = new Map(
    receipts
      .filter((receipt) => receipt.acknowledged_at)
      .map((receipt) => [receipt.id, receipt.acknowledged_at]),
  );
  for (const item of items) {
    if (
      item.type === "human" &&
      item.guidanceId &&
      acknowledged.has(item.guidanceId)
    ) {
      item.incorporatedAt =
        acknowledged.get(item.guidanceId) ?? item.incorporatedAt;
    }
  }
  const active = [...openRuns.entries()].pop();
  return {
    items,
    activeRunId: active?.[0] ?? null,
    activeLabel: active?.[1].label ?? null,
    activeSince: active?.[1].at ?? null,
  };
}

export function summarizeSteps(steps: Step[]): string {
  const counts = new Map<string, number>();
  for (const step of steps) {
    const noun =
      step.kind === "knowledge"
        ? "knowledge update"
        : step.kind === "status"
          ? "update"
          : /^(bash|command)$/i.test(step.tool)
            ? "command"
            : /^(edit|write|multiedit|notebookedit)$/i.test(step.tool)
              ? "edit"
              : /^(read|glob|grep|ls)$/i.test(step.tool)
                ? "read"
                : /search|fetch/i.test(step.tool)
                  ? "web lookup"
                  : /subagent|task|agent/i.test(step.tool)
                    ? "subagent"
                    : "tool call";
    counts.set(noun, (counts.get(noun) ?? 0) + 1);
  }
  return [...counts.entries()]
    .map(([noun, count]) => `${count} ${noun}${count === 1 ? "" : "s"}`)
    .join(", ");
}
