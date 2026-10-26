import type { Engine, ProjectStatus } from "./types";

const relative = new Intl.RelativeTimeFormat(undefined, { numeric: "auto", style: "short" });
const time = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });
const dayTime = new Intl.DateTimeFormat(undefined, {
  month: "short",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
});
const day = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", year: "numeric" });
const compact = new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 });

export function timeAgo(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) return "—";
  const seconds = Math.round((new Date(iso).getTime() - now) / 1000);
  const abs = Math.abs(seconds);
  if (abs < 45) return "just now";
  if (abs < 3600) return relative.format(Math.round(seconds / 60), "minute");
  if (abs < 86_400) return relative.format(Math.round(seconds / 3600), "hour");
  if (abs < 86_400 * 30) return relative.format(Math.round(seconds / 86_400), "day");
  return day.format(new Date(iso));
}

export function clockTime(iso: string): string {
  const date = new Date(iso);
  const today = new Date();
  return date.toDateString() === today.toDateString() ? time.format(date) : dayTime.format(date);
}

export function fullDate(iso: string | null | undefined): string {
  if (!iso) return "—";
  return new Date(iso).toLocaleString();
}

export function shortDate(iso: string | null | undefined): string {
  if (!iso) return "—";
  return day.format(new Date(iso));
}

export function duration(ms: number | null | undefined): string {
  if (ms == null) return "—";
  if (ms < 1000) return `${ms}ms`;
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  if (minutes < 60) return rest ? `${minutes}m ${rest}s` : `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${minutes % 60}m`;
}

export function tokens(value: number | null | undefined): string {
  if (value == null) return "—";
  return compact.format(value);
}

export function cost(microusd: number | null | undefined): string {
  if (microusd == null) return "—";
  const dollars = microusd / 1_000_000;
  if (dollars > 0 && dollars < 0.01) return "<$0.01";
  return `$${dollars.toFixed(2)}`;
}

export function percent(value: number | null | undefined): string {
  if (value == null) return "—";
  return `${Math.round(value * 100)}%`;
}

export function engineLabel(engine: Engine | string): string {
  return engine === "claude-code" ? "Claude" : engine === "codex" ? "Codex" : engine;
}

export type Tone = "neutral" | "info" | "success" | "warning" | "critical";

export interface StatusMeta {
  label: string;
  tone: Tone;
  pulse?: boolean;
  description: string;
}

/** Human status for a project; WAITING splits into "needs you" vs idle using the blocker. */
export function projectStatus(status: ProjectStatus, blocker?: string): StatusMeta {
  const blocked = Boolean(blocker && blocker.trim() && blocker.trim().toLowerCase() !== "none");
  switch (status) {
    case "RUNNING":
      return { label: "Working", tone: "info", pulse: true, description: "Limina is advancing the mission." };
    case "WAITING":
      return blocked
        ? { label: "Needs you", tone: "warning", description: "Limina is waiting for your input." }
        : { label: "Idle", tone: "neutral", description: "Limina is waiting for the next wake-up or message." };
    case "PAUSED":
      return { label: "Paused", tone: "neutral", description: "Work is paused until you resume it." };
    case "STOPPED":
      return { label: "Stopped", tone: "neutral", description: "The project is stopped." };
    case "FAILED":
      return { label: "Failed", tone: "critical", description: "The runtime needs intervention." };
    case "COMPLETE":
      return { label: "Complete", tone: "success", description: "The mission reported completion." };
    case "CREATED":
      return { label: "Draft", tone: "neutral", description: "Not started yet." };
    case "ARCHIVED":
      return { label: "Archived", tone: "neutral", description: "Read-only history." };
    default:
      return { label: String(status), tone: "neutral", description: "" };
  }
}

export function isBlocked(blocker: string | null | undefined): boolean {
  const value = blocker?.trim();
  return Boolean(value && value.toLowerCase() !== "none");
}

export function artifactTone(status: string): Tone {
  const value = status.toUpperCase();
  if (["CONFIRMED", "SUPPORTED", "COMPLETED", "PUBLISHED", "VALIDATED"].includes(value)) return "success";
  if (["REJECTED", "REFUTED", "FAILED", "INVALID"].includes(value)) return "critical";
  if (["TESTING", "RUNNING", "IN_PROGRESS"].includes(value)) return "info";
  if (["INCONCLUSIVE", "BLOCKED"].includes(value)) return "warning";
  return "neutral";
}

export function titleCase(value: string): string {
  return value
    .toLowerCase()
    .replace(/[_-]+/g, " ")
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
}

export function slugify(value: string): string {
  return value
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 80);
}
