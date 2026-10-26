import { Menu, Search } from "@carbon/icons-react";
import { Fragment } from "react";
import { Link, useMatch } from "react-router";
import { useProjectStatus } from "../../lib/queries";
import { LifecycleControls } from "../project/Lifecycle";
import { ProjectStatusTag } from "../ui/Status";
import { useCurrentCrumbs } from "./Crumbs";

function ProjectActions({ slug }: { slug: string }) {
  const status = useProjectStatus(slug);
  const project = status.data?.project;
  if (!project) return null;
  return (
    <>
      <ProjectStatusTag status={project.status} blocker={project.blocker} />
      <LifecycleControls project={project} size="sm" />
    </>
  );
}

export function Topbar({ onMenu, onSearch }: { onMenu: () => void; onSearch: () => void }) {
  const crumbs = useCurrentCrumbs();
  const slug = useMatch("/projects/:slug/*")?.params.slug;
  const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);
  return (
    <header className="topbar">
      <button className="btn btn-ghost btn-icon topbar-menu" onClick={onMenu} aria-label="Open navigation">
        <Menu size={20} />
      </button>
      <nav className="breadcrumbs" aria-label="Breadcrumb">
        {crumbs.map((crumb, index) => {
          const last = index === crumbs.length - 1;
          return (
            <Fragment key={`${crumb.label}-${index}`}>
              {index > 0 && <span className={`sep${last ? "" : " crumb-parent-sep"}`}>/</span>}
              {last || !crumb.to ? (
                <span aria-current={last ? "page" : undefined} className={last ? undefined : "crumb-parent"}>
                  {crumb.label}
                </span>
              ) : (
                <Link to={crumb.to} className="crumb-parent">
                  {crumb.label}
                </Link>
              )}
            </Fragment>
          );
        })}
      </nav>
      <div className="topbar-actions">
        <button className="search-trigger" onClick={onSearch} aria-label="Search (⌘K)">
          <Search size={16} />
          <span className="search-label">Search…</span>
          <kbd className="kbd">{isMac ? "⌘K" : "Ctrl K"}</kbd>
        </button>
        {slug && <ProjectActions slug={slug} />}
      </div>
    </header>
  );
}
