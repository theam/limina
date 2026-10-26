import { useSharedProjects, type SharedProject } from "../lib/sharing";
import { Add, Search } from "@carbon/icons-react";
import { useMemo, useState } from "react";
import { Link } from "react-router";
import { useCrumbs } from "../components/shell/Crumbs";
import { ProjectAvatar } from "../components/shell/ProjectSwitcher";
import { Empty, Skeleton } from "../components/ui/Empty";
import { ProjectStatusTag } from "../components/ui/Status";
import { Time } from "../components/ui/Time";
import { engineLabel, isBlocked } from "../lib/format";
import { useProjects } from "../lib/queries";
import type { Project } from "../lib/types";

function rank(project: Project): number {
  if (project.status === "WAITING" && isBlocked(project.blocker)) return 0;
  if (project.status === "FAILED") return 1;
  if (project.status === "RUNNING") return 2;
  if (project.status === "WAITING") return 3;
  return 4;
}

function ProjectCard({ project, shared }: { project: Project; shared?: SharedProject }) {
  const needsYou = project.status === "WAITING" && isBlocked(project.blocker);
  return (
    <Link to={`/projects/${project.slug}`} className="project-card" data-attention={needsYou || undefined}>
      <div className="project-card-head">
        <ProjectAvatar project={project} />
        <div style={{ minWidth: 0, flex: 1 }}>
          <div className="project-card-name">{project.name}</div>
          <div className="project-card-meta">
            {project.slug} · {engineLabel(project.runtime)}
          </div>
        </div>
        <ProjectStatusTag status={project.status} blocker={project.blocker} />
      </div>
      {shared && <span className="muted" style={{ fontSize: 12 }}>Shared by {shared.owners.map(owner => owner.display_name).join(", ") || "a project owner"}</span>}
      <p className="project-card-objective">{project.current_objective || project.mission}</p>
      <div className="project-card-foot">
        <span className="project-card-next">
          {needsYou ? (
            <>
              <span className="label" style={{ color: "rgb(var(--warning))" }}>Needs you</span> {project.blocker}
            </>
          ) : project.next_step ? (
            <>
              <span className="label">Next</span> {project.next_step}
            </>
          ) : (
            <span className="muted">Not started</span>
          )}
        </span>
        <Time iso={project.updated_at} className="muted mono" />
      </div>
    </Link>
  );
}

export function Projects() {
  useCrumbs([{ label: "Projects" }]);
  const projects = useProjects();
  const shared = useSharedProjects();
  const [filter, setFilter] = useState("");
  const items = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    return (projects.data?.items ?? [])
      .filter((item) => !needle || `${item.name} ${item.slug} ${item.mission}`.toLowerCase().includes(needle))
      .sort((a, b) => rank(a) - rank(b) || b.updated_at.localeCompare(a.updated_at));
  }, [projects.data, filter]);

  return (
    <div className="page">
      <div className="page-header">
        <div>
          <h1 className="page-title">Projects</h1>
          <p className="page-subtitle">Long-running missions that Limina runs on Codex or Claude.</p>
        </div>
        <div className="page-actions">
          <Link to="/new" className="btn btn-primary">
            <Add size={16} /> New project
          </Link>
        </div>
      </div>

      {projects.isPending ? (
        <div className="project-grid">
          {[0, 1, 2].map((index) => (
            <div key={index} className="project-card" style={{ gap: 14 }}>
              <Skeleton width="60%" height={18} />
              <Skeleton height={14} />
              <Skeleton width="40%" height={12} />
            </div>
          ))}
        </div>
      ) : (projects.data?.items.length ?? 0) === 0 ? (
        <Empty
          title="Start your first project"
          action={
            <Link to="/new" className="btn btn-primary">
              <Add size={16} /> New project
            </Link>
          }
        >
          Describe a mission and how you’ll judge success. Limina runs it continuously, records hypotheses,
          experiments and findings, and asks you when it needs a decision.
        </Empty>
      ) : (
        <>
          <div className="toolbar">
            <label className="search-field">
              <Search size={16} />
              <input
                className="input"
                placeholder="Filter projects…"
                value={filter}
                onChange={(event) => setFilter(event.target.value)}
              />
            </label>
          </div>
          {items.length ? (
            <div className="project-grid">
              {items.map((project) => (
                <ProjectCard key={project.slug} project={project} shared={shared.data?.find(item => item.slug === project.slug)} />
              ))}
            </div>
          ) : (
            <p className="muted" style={{ padding: "32px 0", textAlign: "center" }}>
              No projects match “{filter}”.
            </p>
          )}
        </>
      )}
    </div>
  );
}
