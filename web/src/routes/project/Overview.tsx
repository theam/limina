import { ArrowRight, Chat, WarningAlt } from "@carbon/icons-react";
import { useMemo } from "react";
import { Link } from "react-router";
import { useCrumbs } from "../../components/shell/Crumbs";
import { Skeleton } from "../../components/ui/Empty";
import { Time } from "../../components/ui/Time";
import { Metrics } from "../../components/project/Metrics";
import { Summary } from "../../components/project/Summary";
import { engineLabel, isBlocked } from "../../lib/format";
import { useKnowledgeList, usePreflight, useProjectStatus, useRuns } from "../../lib/queries";
import { buildTranscript, summarizeSteps } from "../../lib/transcript";
import { useFeedState, useProjectContext } from "./ProjectContext";
import { KnowledgeRow } from "./KnowledgeHome";

function LatestActivity({ slug }: { slug: string }) {
  const feed = useFeedState();
  const items = useMemo(() => buildTranscript(feed.events.slice(-120)).items.slice(-4).reverse(), [feed.events]);
  if (!feed.loaded) return <Skeleton height={180} />;
  if (!items.length) {
    return (
      <div className="panel panel-body muted">
        Nothing has happened yet. Start the project and Limina’s work will stream here.
      </div>
    );
  }
  return (
    <div className="panel">
      {items.map((item) => {
        let icon = "·";
        let text: React.ReactNode = "";
        switch (item.type) {
          case "message":
          case "plan":
            icon = "L";
            text = item.text.split("\n").find((line) => line.trim())?.replace(/[#*_`>]/g, "") ?? "";
            break;
          case "checkpoint":
            icon = "✓";
            text = `Checkpoint — ${item.summary}`;
            break;
          case "human":
            icon = "→";
            text = `${item.actor}: ${item.body}`;
            break;
          case "activity":
            icon = "›";
            text = summarizeSteps(item.steps);
            break;
          case "artifact":
            icon = "◆";
            text = `${item.action} ${item.artifactId}`;
            break;
          case "turn":
            icon = "—";
            text = item.focus ? `Turn started: ${item.focus}` : "Turn started";
            break;
          case "notice":
            icon = "!";
            text = item.text;
            break;
        }
        return (
          <Link key={item.key} to={`/projects/${slug}/conversation`} className="row-link activity-row">
            <span className="activity-row-icon" data-type={item.type} aria-hidden="true">
              {icon}
            </span>
            <span className="activity-row-text">{text}</span>
            <Time iso={item.at} className="muted mono activity-row-time" />
          </Link>
        );
      })}
    </div>
  );
}


export function Overview() {
  const { slug } = useProjectContext();
  const status = useProjectStatus(slug);
  const preflight = usePreflight(slug);
  const findings = useKnowledgeList(slug, { kind: "F", limit: 3 });
  const runs = useRuns(slug, 1);
  const project = status.data?.project;
  useCrumbs([{ label: project?.name ?? slug }]);
  if (!project || !status.data) return <div className="page stack"><Skeleton width="40%" height={32} /><Skeleton height={150} /><Skeleton height={240} /></div>;
  const blocked = isBlocked(project.blocker);
  const warnings = (preflight.data?.checks ?? []).filter((check) => check.status === "FAIL" || check.status === "WARN");
  const outcome = runs.data?.items[0];
  const base = `/projects/${slug}`;

  return (
    <div className="page overview">
      <div className="page-header">
        <div className="overview-heading">
          <h1 className="page-title">{project.name}</h1>
          <Link className="section-link overview-mission-link" to={`${base}/knowledge/CHALLENGE`}>Read mission and success criteria <ArrowRight size={14} aria-hidden="true" /></Link>
        </div>
        <div className="page-actions"><Link to={`${base}/conversation`} className="btn btn-primary"><Chat size={16} aria-hidden="true" />Open conversation</Link></div>
      </div>

      {blocked ? <section className="notice overview-attention" data-tone="warning" aria-label="Needs your attention">
        <WarningAlt size={18} aria-hidden="true" /><div><Summary label="Needs your attention" readLabel="Read full request" source={project.blocker} slug={slug} /><Link className="btn btn-sm" to={`${base}/conversation`}>Respond in conversation <ArrowRight size={14} aria-hidden="true" /></Link></div>
      </section> : warnings.length > 0 && <section className="overview-attention stack" aria-label="Before research can proceed">
        {warnings.map((check) => <div key={check.name} className="notice" data-tone={check.status === "FAIL" ? "critical" : "warning"}>
          <WarningAlt size={16} aria-hidden="true" /><div>
            <p>{check.name === "runtime" ? `Connect a ${engineLabel(project.runtime)} account before starting research.` : check.detail}</p>
            {project.role === "OWNER" && <Link className="section-link" to={check.name === "runtime" ? `${base}/settings/engine` : `${base}/settings`}>{check.name === "runtime" ? "Connect account" : "Review project settings"} <ArrowRight size={14} aria-hidden="true" /></Link>}
          </div>
        </div>)}
      </section>}

      <Metrics project={project} />

      <section className="section" aria-label="Current focus and next action">
        <div className="overview-focus">
          <Summary label="Current objective" source={project.current_objective} slug={slug} fallback="Review the mission before starting research." />
          <Summary label="Next step" source={project.next_step} slug={slug} fallback="Start the project when its mission and prerequisites are ready." />
        </div>
      </section>

      <section className="section" aria-labelledby="latest-outcome">
        <div className="section-header"><h2 className="section-title" id="latest-outcome">Latest outcome</h2><Link className="section-link" to={`${base}/conversation`}>Read conversation <ArrowRight size={14} aria-hidden="true" /></Link></div>
        {runs.isPending ? <Skeleton height={100} /> : runs.isError ? <div className="panel panel-body"><p>Couldn’t load the latest outcome.</p><button className="btn btn-sm" onClick={() => runs.refetch()}>Try again</button></div> : outcome?.summary ? <div className="panel panel-body"><Summary label="Recorded result" source={outcome.summary} slug={slug} /><p className="field-hint">Recorded <Time iso={outcome.completed_at ?? outcome.started_at} />. KPI measurements above assess progress toward the targets.</p></div> : <div className="panel panel-body muted">No research outcome has been recorded yet.</div>}
      </section>

      <div className="grid-2 section">
        <section><div className="section-header"><h2 className="section-title">Recent evidence</h2><Link className="section-link" to={`${base}/knowledge/findings`}>All findings <ArrowRight size={14} aria-hidden="true" /></Link></div>
          {findings.isPending ? <Skeleton height={160} /> : findings.isError ? <div className="panel panel-body"><p>Couldn’t load recent evidence.</p><button className="btn btn-sm" onClick={() => findings.refetch()}>Try again</button></div> : findings.data?.items.length ? <div className="panel">{findings.data.items.map((artifact) => <KnowledgeRow key={artifact.id} artifact={artifact} slug={slug} />)}</div> : <div className="panel panel-body muted">Findings will appear here when experiments establish evidence.</div>}
        </section>
        <section><div className="section-header"><h2 className="section-title">Recent activity</h2><Link className="section-link" to={`${base}/conversation`}>All activity <ArrowRight size={14} aria-hidden="true" /></Link></div><LatestActivity slug={slug} /></section>
      </div>
    </div>
  );
}
