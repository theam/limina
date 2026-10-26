import { Add, Edit } from "@carbon/icons-react";
import * as Dialog from "@radix-ui/react-dialog";
import { useEffect, useRef, useState } from "react";
import { Link } from "react-router";
import { api, ApiError } from "../../lib/api";
import { metricAssessment, metricValue } from "../../lib/metrics";
import { keys, queryClient, useMetrics } from "../../lib/queries";
import type { Metric, MetricInput, Project } from "../../lib/types";
import { Empty, Skeleton } from "../ui/Empty";
import { Tag } from "../ui/Status";
import { Time } from "../ui/Time";
import { Confirm } from "../ui/Confirm";

function MetricEditor({ project, metric, onSaved, onClose, onDirty, onPending }: { project: Project; metric: Metric | null; onSaved: () => void; onClose: () => void; onDirty: (value: boolean) => void; onPending: (value: boolean) => void }) {
  const [metricId] = useState(() => metric?.id ?? crypto.randomUUID());
  const [name, setName] = useState(metric?.name ?? "");
  const [criterion, setCriterion] = useState(metric?.criterion ?? "");
  const [unit, setUnit] = useState(metric?.unit ?? "");
  const [direction, setDirection] = useState<MetricInput["direction"]>(metric?.direction ?? "at_least");
  const [target, setTarget] = useState(metric?.target.toString() ?? "");
  const [baseline, setBaseline] = useState(metric?.baseline?.toString() ?? "");
  const [current, setCurrent] = useState(metric?.current?.toString() ?? "");
  const [evidence, setEvidence] = useState(metric?.evidence_id ?? "");
  const [observed, setObserved] = useState((metric?.observed_at ?? new Date().toISOString()).slice(0, 16));
  const [notes, setNotes] = useState(metric?.notes ?? "");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  const initial = useRef(JSON.stringify([name, criterion, unit, direction, target, baseline, current, evidence, observed, notes]));
  useEffect(() => onDirty(JSON.stringify([name, criterion, unit, direction, target, baseline, current, evidence, observed, notes]) !== initial.current), [name, criterion, unit, direction, target, baseline, current, evidence, observed, notes, onDirty]);
  useEffect(() => onPending(pending), [pending, onPending]);
  const errorRef = useRef<HTMLDivElement>(null);
  useEffect(() => { if (error) errorRef.current?.focus(); }, [error]);
  const save = async (event: React.SubmitEvent<HTMLFormElement>) => {
    event.preventDefault();
    setPending(true); setError(null);
    try {
      await api.setMetric(project.slug, metricId, {
        name, criterion, unit, direction, target: Number(target), baseline: baseline === "" ? null : Number(baseline),
        current: current === "" ? null : Number(current), evidence_id: current === "" ? null : evidence,
        observed_at: current === "" || !observed ? null : new Date(`${observed}:00Z`).toISOString(), notes,
      }, metric?.version ?? 0);
      await queryClient.invalidateQueries({ queryKey: keys.metrics(project.slug) });
      onSaved();
    } catch (failure) { setError(failure as Error); } finally { setPending(false); }
  };
  return (
    <form onSubmit={save}>
      <div className="dialog-header">
        <Dialog.Title className="dialog-title">{metric ? "Edit KPI" : "Add KPI"}</Dialog.Title>
        <Dialog.Description className="dialog-description">Define a measurable target. A current value needs a finding or completed experiment as evidence.</Dialog.Description>
      </div>
      <div className="dialog-body stack">
        <label className="field"><span className="field-label">KPI name</span><input name="metric-name" className="input" value={name} onChange={(e) => setName(e.target.value)} required maxLength={120} autoComplete="off" /></label>
        <label className="field"><span className="field-label">Success criterion</span><textarea name="metric-criterion" className="textarea" rows={2} value={criterion} onChange={(e) => setCriterion(e.target.value)} maxLength={2000} /></label>
        <div className="grid-2">
          <label className="field"><span className="field-label">Target</span><input name="metric-target" className="input" type="number" step="any" value={target} onChange={(e) => setTarget(e.target.value)} required /></label>
          <label className="field"><span className="field-label">Unit</span><input name="metric-unit" className="input" placeholder="%, ms, points…" value={unit} onChange={(e) => setUnit(e.target.value)} maxLength={40} autoComplete="off" /></label>
          <label className="field"><span className="field-label">Target direction</span><select className="select" value={direction} onChange={(e) => setDirection(e.target.value as MetricInput["direction"])}><option value="at_least">At least (higher is better)</option><option value="at_most">At most (lower is better)</option></select></label>
          <label className="field"><span className="field-label">Baseline · optional</span><input name="metric-baseline" className="input" type="number" step="any" value={baseline} onChange={(e) => setBaseline(e.target.value)} /></label>
        </div>
        <fieldset className="metric-measurement"><legend>Latest measurement · optional</legend>
          <div className="grid-2">
            <label className="field"><span className="field-label">Current value</span><input name="metric-current" className="input" type="number" step="any" value={current} onChange={(e) => setCurrent(e.target.value)} /></label>
            <label className="field"><span className="field-label">Observed at (UTC)</span><input name="metric-observed" className="input" type="datetime-local" value={observed} onChange={(e) => setObserved(e.target.value)} disabled={current === ""} required={current !== ""} /></label>
          </div>
          <label className="field"><span className="field-label">Evidence note ID</span><input name="metric-evidence" className="input mono" placeholder="F001 or E002…" value={evidence} onChange={(e) => setEvidence(e.target.value)} required={current !== ""} maxLength={200} autoComplete="off" spellCheck={false} disabled={current === ""} /></label>
          <Link className="link" to={`/projects/${project.slug}/knowledge/findings`} target="_blank" rel="noreferrer">Find evidence in a new tab</Link>
        </fieldset>
        <label className="field"><span className="field-label">Measurement notes · optional</span><textarea name="metric-notes" className="textarea" rows={2} value={notes} onChange={(e) => setNotes(e.target.value)} maxLength={4000} /></label>
        {error && <div className="notice" data-tone="critical" role="alert" tabIndex={-1} ref={errorRef}>
          <div>{error.message}{error instanceof ApiError && error.status === 409 && <button type="button" className="btn btn-sm" onClick={() => { queryClient.invalidateQueries({ queryKey: keys.metrics(project.slug) }); onClose(); }}>Close and load latest</button>}</div>
        </div>}
      </div>
      <div className="dialog-footer"><button type="button" className="btn" onClick={onClose} disabled={pending}>Cancel</button><button className="btn btn-primary" type="submit" disabled={pending}>{pending ? "Saving…" : "Save KPI"}</button></div>
    </form>
  );
}

function MetricCard({ metric, slug, edit }: { metric: Metric; slug: string; edit?: () => void }) {
  const assessment = metricAssessment(metric);
  return (
    <article className="metric-card">
      <div className="cluster"><h3>{metric.name}</h3>{metric.current != null && <Tag tone={assessment.met ? "success" : "neutral"}>{assessment.label}</Tag>}{edit && <button className="btn btn-ghost btn-icon" aria-label={`Edit ${metric.name}`} onClick={edit}><Edit size={16} aria-hidden="true" /></button>}</div>
      {metric.criterion && <details className="metric-context"><summary>Success criterion</summary><p>{metric.criterion}</p></details>}
      <dl className="metric-values"><div><dt>Current</dt><dd>{metricValue(metric.current, metric.unit)}</dd></div><div><dt>Target</dt><dd>{metric.direction === "at_least" ? "≥ " : "≤ "}{metricValue(metric.target, metric.unit)}</dd></div><div><dt>Baseline</dt><dd>{metric.baseline == null ? "Not set" : metricValue(metric.baseline, metric.unit)}</dd></div></dl>
      {assessment.progress != null && <div className="metric-progress"><progress value={assessment.progress} max={100} aria-label={`${metric.name}: progress from baseline to target`} /><span>{Math.round(assessment.progress)}% from baseline to target</span></div>}
      {metric.current == null ? <p className="muted">Record a measurement to assess this target.</p> : <p className="muted">Observed <Time iso={metric.observed_at} /> · <Link className="link" to={`/projects/${slug}/knowledge/${metric.evidence_id}`}>Evidence {metric.evidence_id}</Link></p>}
      {metric.notes && <details className="metric-context"><summary>Measurement notes</summary><p>{metric.notes}</p></details>}
    </article>
  );
}

export function Metrics({ project }: { project: Project }) {
  const metrics = useMetrics(project.slug);
  const [editor, setEditor] = useState<{ metric: Metric | null } | null>(null);
  const [dirty, setDirty] = useState(false);
  const [pending, setPending] = useState(false);
  const [discard, setDiscard] = useState(false);
  const close = () => { if (pending) return; if (dirty) setDiscard(true); else setEditor(null); };
  useEffect(() => {
    if (!editor || !dirty) return;
    const warn = (event: BeforeUnloadEvent) => { event.preventDefault(); event.returnValue = ""; };
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [editor, dirty]);
  const editable = project.role !== "VIEWER" && project.status !== "ARCHIVED";
  return (
    <section className="section" aria-labelledby="mission-progress">
      <div className="section-header"><div><h2 className="section-title" id="mission-progress">Mission progress</h2><p className="field-hint">Measured outcomes against the project’s targets.</p></div>{editable && <button className="btn btn-sm" onClick={() => setEditor({ metric: null })}><Add size={14} aria-hidden="true" />Add KPI</button>}</div>
      {metrics.isPending ? <Skeleton height={150} /> : metrics.isError ? <Empty title="Couldn’t load KPIs" action={<button className="btn" onClick={() => metrics.refetch()}>Try again</button>} /> : metrics.data?.length ? <>
        <div className="metric-grid">{metrics.data.slice(0, 6).map((metric) => <MetricCard key={metric.id} metric={metric} slug={project.slug} edit={editable ? () => setEditor({ metric }) : undefined} />)}</div>
        {metrics.data.length > 6 && <details className="section"><summary>Show {metrics.data.length - 6} more KPIs</summary><div className="metric-grid">{metrics.data.slice(6).map((metric) => <MetricCard key={metric.id} metric={metric} slug={project.slug} edit={editable ? () => setEditor({ metric }) : undefined} />)}</div></details>}
      </> : <div className="panel panel-body"><h3>No KPIs defined yet</h3><p className="muted">Progress is not measured yet. Define targets from the success criteria and attach evidence when you record results.</p><Link className="section-link" to={`/projects/${project.slug}/knowledge/CHALLENGE#success-criteria`}>Read success criteria</Link></div>}
      <Dialog.Root open={Boolean(editor)} onOpenChange={(open) => { if (!open) close(); }}><Dialog.Portal><Dialog.Overlay className="overlay" /><Dialog.Content className="dialog metric-editor">{editor && <MetricEditor key={editor.metric?.id ?? "new"} project={project} metric={editor.metric} onSaved={() => { setDirty(false); setPending(false); setEditor(null); }} onClose={close} onDirty={setDirty} onPending={setPending} />}</Dialog.Content></Dialog.Portal></Dialog.Root>
      <Confirm open={discard} onOpenChange={setDiscard} title="Discard KPI changes?" body="Your unsaved changes will be lost." action="Discard changes" onConfirm={() => { setDiscard(false); setDirty(false); setEditor(null); }} />
    </section>
  );
}
