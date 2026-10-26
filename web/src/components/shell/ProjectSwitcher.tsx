import { Add, Checkmark, ChevronSort, Grid } from "@carbon/icons-react";
import * as Popover from "@radix-ui/react-popover";
import { Command } from "cmdk";
import { useState } from "react";
import { useLocation, useNavigate } from "react-router";
import { engineLabel, projectStatus } from "../../lib/format";
import { useProjects } from "../../lib/queries";
import type { Project } from "../../lib/types";
import { Dot } from "../ui/Status";

export function initials(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  return (words.length > 1 ? words[0][0] + words[1][0] : name.slice(0, 2)) || "·";
}

export function ProjectAvatar({ project, size }: { project: Pick<Project, "name">; size?: "sm" }) {
  return (
    <span className="project-avatar" data-size={size} aria-hidden="true">
      {initials(project.name)}
    </span>
  );
}

/** Vercel-style scope switcher: search, jump between projects, keep the current section. */
export function ProjectSwitcher({ slug, onNavigate }: { slug?: string; onNavigate?: () => void }) {
  const [open, setOpen] = useState(false);
  const projects = useProjects();
  const navigate = useNavigate();
  const location = useLocation();
  const items = projects.data?.items ?? [];
  const current = items.find((item) => item.slug === slug);
  const section = slug ? location.pathname.split("/")[3] : undefined;

  const go = (path: string) => {
    setOpen(false);
    onNavigate?.();
    navigate(path);
  };


  return (
    <Popover.Root open={open} onOpenChange={setOpen}>
      <Popover.Trigger className="switcher" aria-label="Switch project">
        {current ? (
          <ProjectAvatar project={current} />
        ) : (
          <span className="project-avatar" style={{ background: "var(--pressed)", color: "rgb(var(--fg))" }}>
            <Grid size={14} />
          </span>
        )}
        <span className="switcher-text">
          <span className="switcher-name">{current?.name ?? (slug ? slug : "All projects")}</span>
          <span className="switcher-meta">
            {current ? engineLabel(current.runtime) : `${items.length} project${items.length === 1 ? "" : "s"}`}
          </span>
        </span>
        <ChevronSort size={16} className="muted" />
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content className="floating switcher-menu" align="start" sideOffset={6}>
          <Command label="Switch project" loop>
            <Command.Input placeholder="Find a project…" autoFocus />
            <Command.List>
              <Command.Empty>No projects match.</Command.Empty>
              <Command.Group heading="Projects">
                {items.map((project) => {
                  const status = projectStatus(project.status, project.blocker);
                  return (
                    <Command.Item
                      key={project.slug}
                      value={`${project.name} ${project.slug}`}
                      className="menu-item"
                      onSelect={() =>
                        go(`/projects/${project.slug}${section && section !== "settings" ? `/${section}` : ""}`)
                      }
                    >
                      <ProjectAvatar project={project} size="sm" />
                      <span className="command-note-title">
                        {project.name}
                      </span>
                      <Dot tone={status.tone} pulse={status.pulse} />
                      <span style={{ width: 16 }}>{project.slug === slug && <Checkmark size={16} />}</span>
                    </Command.Item>
                  );
                })}
              </Command.Group>
            </Command.List>
            <div className="menu-footer">
              <Command.Item className="menu-item" onSelect={() => go("/new")} value="__new">
                <Add size={16} /> New project
              </Command.Item>
              <Command.Item className="menu-item" onSelect={() => go("/projects")} value="__all">
                <Grid size={16} /> All projects
              </Command.Item>
            </div>
          </Command>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}
