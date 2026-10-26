import {
  Book,
  CheckmarkFilled,
  CheckmarkOutline,
  ChevronDown,
  ChevronRight,
  Chip,
  CloseFilled,
  Code,
  DocumentView,
  Edit,
  Flag,
  Information,
  Search,
  Terminal,
  Time as TimeIcon,
  Tools,
  WarningAltFilled,
} from "@carbon/icons-react";
import * as HoverCard from "@radix-ui/react-hover-card";
import { memo, useState } from "react";
import { Link } from "react-router";
import { clockTime, cost, duration, engineLabel, fullDate, tokens } from "../../lib/format";
import { summarizeSteps, type Step, type TranscriptItem } from "../../lib/transcript";
import type { Engine, Run } from "../../lib/types";
import type { Vault } from "../../lib/vault";
import { DocPreview, Markdown, isArtifactKey } from "../knowledge/Markdown";
import { LiminaMark } from "../ui/Logo";
import { Tag } from "../ui/Status";
import { Elapsed } from "../ui/Time";

type ItemOf<T extends TranscriptItem["type"]> = Extract<TranscriptItem, { type: T }>;

function Stamp({ at }: { at: string }) {
  return (
    <time className="stamp" dateTime={at} title={fullDate(at)}>
      {clockTime(at)}
    </time>
  );
}

/* ─── Turn divider ──────────────────────────────────────────────────── */

export function TurnDivider({
  item,
  run,
  number,
  active,
}: {
  item: ItemOf<"turn">;
  run?: Run;
  number?: number;
  active: boolean;
}) {
  return (
    <div className="turn" role="separator">
      <div className="turn-line">
        <span className="turn-label">
          {number ? `Turn ${number}` : "Turn"} · <Stamp at={item.at} />
        </span>
        <span className="turn-rule" />
        <span className="turn-stats">
          {active ? (
            <>
              <span className="spinner" style={{ width: 10, height: 10 }} /> <Elapsed since={item.at} />
            </>
          ) : run ? (
            <>
              {duration(run.duration_ms)}
              {run.usage.total_tokens != null && <> · {tokens(run.usage.total_tokens)} tokens</>}
              {run.usage.cost_microusd != null && <> · {cost(run.usage.cost_microusd)}</>}
            </>
          ) : null}
        </span>
      </div>
      {item.focus && <div className="turn-focus">{item.focus}</div>}
    </div>
  );
}

/* ─── Agent message ─────────────────────────────────────────────────── */

export const AgentMessage = memo(function AgentMessage({
  item,
  engine,
  slug,
  vault,
}: {
  item: ItemOf<"message"> | ItemOf<"plan">;
  engine: Engine;
  slug: string;
  vault?: Vault;
}) {
  return (
    <div className="msg msg-agent">
      <div className="msg-avatar" aria-hidden="true">
        <LiminaMark size={24} />
      </div>
      <div className="msg-main">
        <div className="msg-meta">
          <span className="msg-author">Limina</span>
          <span className="msg-engine">{engineLabel(engine)}</span>
          {item.type === "plan" && <Tag>Plan</Tag>}
          <Stamp at={item.at} />
        </div>
        <Markdown source={item.text} slug={slug} vault={vault} autolink className="msg-body" />
      </div>
    </div>
  );
});

/* ─── Tool activity ─────────────────────────────────────────────────── */

function StepIcon({ step }: { step: Step }) {
  const tool = step.tool.toLowerCase();
  if (step.kind === "knowledge") return <Book size={16} />;
  if (step.kind === "status") return <Information size={16} />;
  if (/^(bash|command)$/.test(tool)) return <Terminal size={16} />;
  if (/^(edit|write|multiedit|notebookedit)$/.test(tool)) return <Edit size={16} />;
  if (/^(read|glob|grep|ls)$/.test(tool)) return <DocumentView size={16} />;
  if (/search|fetch/.test(tool)) return <Search size={16} />;
  if (/subagent|task|agent/.test(tool)) return <Chip size={16} />;
  if (/^mcp/.test(tool)) return <Code size={16} />;
  return <Tools size={16} />;
}

function StepState({ step }: { step: Step }) {
  if (step.state === "running") return <span className="spinner" style={{ width: 12, height: 12 }} aria-label="Running" />;
  if (step.state === "failed")
    return (
      <span className="step-state" data-tone="critical">
        <CloseFilled size={14} />
        {step.exitCode != null ? `exit ${step.exitCode}` : "failed"}
      </span>
    );
  if (step.state === "declined")
    return (
      <span className="step-state" data-tone="warning">
        <WarningAltFilled size={14} /> declined
      </span>
    );
  if (step.state === "stopped") return <span className="step-state">stopped</span>;
  if (step.state === "ok") return <CheckmarkOutline size={14} className="step-ok" aria-label="Done" />;
  return null;
}

function stepText(step: Step): string {
  if (step.kind === "knowledge") return "Updating durable project knowledge";
  if (step.files.length) return step.files.map((file) => file.path).join(", ");
  return step.input || step.summary || "";
}

export function ActivityGroup({ item }: { item: ItemOf<"activity"> }) {
  const running = item.steps.some((step) => step.state === "running");
  const failed = item.steps.filter((step) => step.state === "failed").length;
  const [open, setOpen] = useState(false);
  const latest = item.steps[item.steps.length - 1];
  const single = item.steps.length === 1;

  return (
    <div className="activity" data-open={open}>
      <button className="activity-summary" onClick={() => setOpen((value) => !value)} aria-expanded={open}>
        {open ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
        {running ? <span className="spinner" style={{ width: 12, height: 12 }} /> : <StepIcon step={latest} />}
        <span className="activity-title">
          {single ? (
            <>
              <span className="activity-tool">{latest.kind === "knowledge" ? "Knowledge" : latest.tool}</span>
              <span className="activity-input">{stepText(latest)}</span>
            </>
          ) : (
            <>
              <span className="activity-count">{summarizeSteps(item.steps)}</span>
              {running && <span className="activity-input">{stepText(latest)}</span>}
            </>
          )}
        </span>
        {failed > 0 && (
          <span className="step-state" data-tone="critical">
            <CloseFilled size={14} /> {failed} failed
          </span>
        )}
        <Stamp at={item.at} />
      </button>
      {open && (
        <ol className="steps">
          {item.steps.map((step) => (
            <li key={step.key} className="step">
              <StepIcon step={step} />
              <span className="step-tool">{step.kind === "knowledge" ? "limina" : step.tool}</span>
              <code className="step-input" title={stepText(step)}>
                {stepText(step)}
              </code>
              {step.durationMs != null && <span className="step-duration">{duration(step.durationMs)}</span>}
              <StepState step={step} />
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}

/* ─── Knowledge write ───────────────────────────────────────────────── */

export function ArtifactEvent({ item, slug, vault }: { item: ItemOf<"artifact">; slug: string; vault?: Vault }) {
  const doc = vault?.resolve(item.artifactId);
  const content = (
    <Link to={`/projects/${slug}/knowledge/${item.artifactId}`} className="artifact-event">
      <Book size={16} />
      <span className="muted">{item.action}</span>
      {isArtifactKey(item.artifactId) && <span className="id-chip">{item.artifactId}</span>}
      <span className="artifact-event-title">{doc?.title ?? ""}</span>
      <Stamp at={item.at} />
    </Link>
  );
  if (!doc) return content;
  return (
    <HoverCard.Root openDelay={350} closeDelay={80}>
      <HoverCard.Trigger asChild>{content}</HoverCard.Trigger>
      <HoverCard.Portal>
        <HoverCard.Content className="floating hover-card" side="top" align="start" sideOffset={6}>
          <DocPreview doc={doc} />
        </HoverCard.Content>
      </HoverCard.Portal>
    </HoverCard.Root>
  );
}

/* ─── Checkpoint ────────────────────────────────────────────────────── */

const CHECKPOINT: Record<string, { label: string; tone: "info" | "warning" | "success" | "neutral" }> = {
  RUNNING: { label: "Continuing", tone: "info" },
  WAITING: { label: "Waiting for direction", tone: "neutral" },
  COMPLETE: { label: "Mission complete", tone: "success" },
  PAUSED: { label: "Paused", tone: "neutral" },
  STOPPED: { label: "Stopped", tone: "neutral" },
};

export function Checkpoint({ item, slug, vault }: { item: ItemOf<"checkpoint">; slug: string; vault?: Vault }) {
  const meta = CHECKPOINT[item.status] ?? { label: item.status || "Checkpoint", tone: "neutral" as const };
  const blocked = item.blocker && item.blocker.trim().toLowerCase() !== "none";
  return (
    <div className="checkpoint">
      <div className="checkpoint-head">
        <Flag size={16} />
        <span className="checkpoint-title">Checkpoint</span>
        <Tag tone={blocked ? "warning" : meta.tone}>{blocked ? "Needs you" : meta.label}</Tag>
        <span className="spacer" />
        <Stamp at={item.at} />
      </div>
      {item.summary && <Markdown source={item.summary} slug={slug} vault={vault} autolink className="checkpoint-body" />}
      {(item.nextStep || blocked) && (
        <dl className="checkpoint-facts">
          {item.nextStep && (
            <>
              <dt>Next</dt>
              <dd>{item.nextStep}</dd>
            </>
          )}
          {blocked && (
            <>
              <dt data-tone="warning">Blocker</dt>
              <dd>{item.blocker}</dd>
            </>
          )}
        </dl>
      )}
    </div>
  );
}

/* ─── Human guidance ────────────────────────────────────────────────── */

const KIND_LABEL: Record<string, string> = {
  ANSWER: "Answer",
  APPROVAL: "Approval",
  BLOCKER: "Blocker",
  INTERRUPT: "Interrupt",
};

export function HumanMessage({
  item,
  delivery,
  onRetry,
}: {
  item: ItemOf<"human">;
  delivery?: "LIVE" | "QUEUED";
  onRetry?: () => void;
}) {
  const state = item.delivery ?? delivery;
  let receipt: React.ReactNode;
  if (item.incorporatedAt) {
    receipt = (
      <span className="receipt" data-tone="success" title={`Incorporated ${fullDate(item.incorporatedAt)}`}>
        <CheckmarkFilled size={12} /> Incorporated
      </span>
    );
  } else if (state === "SENDING") {
    receipt = <span className="receipt">Sending…</span>;
  } else if (state === "FAILED") {
    receipt = (
      <span className="receipt" data-tone="critical">
        Not sent ·{" "}
        <button className="link" onClick={onRetry}>
          Retry
        </button>
      </span>
    );
  } else if (state === "LIVE") {
    receipt = (
      <span className="receipt" title="Delivered into the active turn. It is acknowledged at the next checkpoint.">
        <CheckmarkOutline size={12} /> Delivered live
      </span>
    );
  } else {
    receipt = (
      <span className="receipt" title="Stored durably and read at the start of the next turn.">
        <TimeIcon size={12} /> Queued for next turn
      </span>
    );
  }
  return (
    <div className="msg msg-human">
      <div className="msg-meta">
        <span className="msg-author">{item.actor}</span>
        {KIND_LABEL[item.kind] && <Tag tone={item.kind === "INTERRUPT" ? "warning" : "neutral"}>{KIND_LABEL[item.kind]}</Tag>}
        <Stamp at={item.at} />
      </div>
      <div className="bubble">{item.body}</div>
      {receipt}
    </div>
  );
}

/* ─── Notices ───────────────────────────────────────────────────────── */

export function Notice({ item }: { item: ItemOf<"notice"> }) {
  return (
    <div className="event-notice" data-tone={item.tone}>
      {item.tone === "critical" || item.tone === "warning" ? <WarningAltFilled size={14} /> : <Information size={14} />}
      <span>
        {item.text}
        {item.detail && <span className="event-notice-detail"> — {item.detail}</span>}
      </span>
      <Stamp at={item.at} />
    </div>
  );
}
