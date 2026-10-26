import {
  Add,
  Book,
  Chat,
  Dashboard,
  Document,
  Grid,
  Moon,
  Plug,
  Settings,
  Sun,
} from "@carbon/icons-react";
import * as Dialog from "@radix-ui/react-dialog";
import { Command } from "cmdk";
import { useEffect, useState } from "react";
import { useMatch, useNavigate } from "react-router";
import { useProjects, useKnowledgeList } from "../../lib/queries";
import { useDebouncedValue } from "../../lib/useDebouncedValue";
import { setTheme } from "../../lib/theme";
import { ArtifactStatusTag } from "../ui/Status";
import { ProjectAvatar } from "./ProjectSwitcher";

function ProjectCommands({ slug, go, search }: { slug: string; go: (path: string) => void; search: string }) {
  const query = useDebouncedValue(search);
  const notes = useKnowledgeList(slug, { query: query || undefined, limit: 8 }, query === search);
  const searching = query !== search || notes.isFetching;
  const base = `/projects/${slug}`;
  return (
    <>
      <Command.Group heading="This project">
        <Command.Item className="menu-item" onSelect={() => go(base)}>
          <Dashboard size={16} /> Overview
        </Command.Item>
        <Command.Item className="menu-item" onSelect={() => go(`${base}/conversation`)} keywords={["chat", "steer", "message"]}>
          <Chat size={16} /> Conversation
        </Command.Item>
        <Command.Item className="menu-item" onSelect={() => go(`${base}/knowledge`)} keywords={["vault", "obsidian", "notes"]}>
          <Book size={16} /> Knowledge
        </Command.Item>
        <Command.Item className="menu-item" onSelect={() => go(`${base}/settings`)}>
          <Settings size={16} /> Project settings
        </Command.Item>
        <Command.Item
          className="menu-item"
          onSelect={() => go(`${base}/settings/engine`)}
          keywords={["account", "token", "sign in", "subscription", "codex", "claude"]}
        >
          <Settings size={16} /> Engine and account
        </Command.Item>
        <Command.Item
          className="menu-item"
          onSelect={() => go(`${base}/settings/connections`)}
          keywords={["mcp", "integrations", "enable", "data"]}
        >
          <Plug size={16} /> Project connections
        </Command.Item>
      </Command.Group>
      <Command.Group heading="Knowledge" forceMount>
        {searching && <p role="status" className="command-search-status">Searching project knowledge…</p>}
        {notes.isError && <p role="status" className="command-search-status">Couldn’t search notes. Open Knowledge to retry.</p>}
        {!searching && !notes.isError && !notes.data?.items.length && <p role="status" className="command-search-status">No research notes match this search.</p>}
        {(!searching ? notes.data?.items ?? [] : []).map((artifact) => (
          <Command.Item forceMount key={artifact.id} value={`note ${artifact.id}`} className="menu-item" onSelect={() => go(`${base}/knowledge/${artifact.id}`)}>
            <Document size={16} aria-hidden="true" /><span className="id-chip">{artifact.id}</span>
            <span className="command-note-title">{artifact.title}</span><ArtifactStatusTag status={artifact.status} />
          </Command.Item>
        ))}
        <Command.Item forceMount className="menu-item" value="search-all-knowledge" onSelect={() => go(`${base}/knowledge${query ? `?q=${encodeURIComponent(query)}` : ""}`)}>
          <Book size={16} aria-hidden="true" />Search all project knowledge
        </Command.Item>
      </Command.Group>
    </>
  );
}

export function CommandMenu({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const navigate = useNavigate();
  const slug = useMatch("/projects/:slug/*")?.params.slug;
  const projects = useProjects();
  const [search, setSearch] = useState("");

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() === "k" && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        onOpenChange(!open);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onOpenChange]);

  useEffect(() => {
    if (!open) setSearch("");
  }, [open]);

  const go = (path: string) => {
    onOpenChange(false);
    navigate(path);
  };

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="overlay" />
        <Dialog.Content className="dialog command" aria-describedby={undefined}>
          <Dialog.Title className="sr-only">Search Limina</Dialog.Title>
          <Command label="Command menu" loop>
            <Command.Input
              value={search}
              onValueChange={setSearch}
              placeholder={slug ? "Search knowledge, pages, projects…" : "Search projects and actions…"}
            />
            <Command.List>
              {slug && <ProjectCommands slug={slug} go={go} search={search} />}
              <Command.Group heading="Projects">
                {(projects.data?.items ?? []).map((project) => (
                  <Command.Item
                    key={project.slug}
                    value={`project ${project.name} ${project.slug}`}
                    className="menu-item"
                    onSelect={() => go(`/projects/${project.slug}`)}
                  >
                    <ProjectAvatar project={project} size="sm" />
                    {project.name}
                  </Command.Item>
                ))}
                <Command.Item className="menu-item" onSelect={() => go("/projects")}>
                  <Grid size={16} /> All projects
                </Command.Item>
                <Command.Item
                  className="menu-item"
                  onSelect={() => go("/connections")}
                  keywords={["mcp", "integrations", "servers", "oauth", "sign in"]}
                >
                  <Plug size={16} /> Connections
                </Command.Item>
              </Command.Group>
              <Command.Group heading="Actions">
                <Command.Item className="menu-item" onSelect={() => go("/new")} keywords={["create", "kickoff"]}>
                  <Add size={16} /> New project
                </Command.Item>
                <Command.Item
                  className="menu-item"
                  onSelect={() => {
                    setTheme("light");
                    onOpenChange(false);
                  }}
                  keywords={["theme", "appearance"]}
                >
                  <Sun size={16} /> Use light theme
                </Command.Item>
                <Command.Item
                  className="menu-item"
                  onSelect={() => {
                    setTheme("dark");
                    onOpenChange(false);
                  }}
                  keywords={["theme", "appearance"]}
                >
                  <Moon size={16} /> Use dark theme
                </Command.Item>
              </Command.Group>
            </Command.List>
          </Command>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
