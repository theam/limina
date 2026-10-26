import type { Tone } from "../../lib/format";
import { artifactTone, projectStatus, titleCase } from "../../lib/format";
import type { ProjectStatus } from "../../lib/types";

export function Dot({ tone, pulse = false }: { tone: Tone; pulse?: boolean }) {
  return <span className="dot" data-tone={tone} data-pulse={pulse} aria-hidden="true" />;
}

export function Tag({ tone = "neutral", children }: { tone?: Tone; children: React.ReactNode }) {
  return (
    <span className="tag" data-tone={tone}>
      {children}
    </span>
  );
}

export function ProjectStatusTag({ status, blocker }: { status: ProjectStatus; blocker?: string }) {
  const meta = projectStatus(status, blocker);
  return (
    <span className="tag" data-tone={meta.tone} title={meta.description}>
      <Dot tone={meta.tone} pulse={meta.pulse} />
      {meta.label}
    </span>
  );
}

export function ArtifactStatusTag({ status }: { status: string | null | undefined }) {
  if (!status) return null;
  return <Tag tone={artifactTone(status)}>{titleCase(status)}</Tag>;
}
