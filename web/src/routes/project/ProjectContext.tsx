import { createContext, useContext, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { Outlet, useParams } from "react-router";
import { ApiError } from "../../lib/api";
import { ProjectFeed } from "../../lib/feed";
import { keys, queryClient, useProjectStatus } from "../../lib/queries";
import type { LiminaEvent, ProjectStatusResponse } from "../../lib/types";
import { Empty } from "../../components/ui/Empty";
import { Link } from "react-router";
import { WarningAlt } from "@carbon/icons-react";

interface ProjectContextValue {
  slug: string;
  feed: ProjectFeed;
}

const ProjectContext = createContext<ProjectContextValue | null>(null);

export function useProjectContext(): ProjectContextValue {
  const value = useContext(ProjectContext);
  if (!value) throw new Error("useProjectContext must be used inside a project route");
  return value;
}

export function useFeedState() {
  const { feed } = useProjectContext();
  return useSyncExternalStore(feed.subscribe, feed.getState, feed.getState);
}

const KNOWLEDGE_EVENT = /^(hypothesis|experiment|finding)\.|^project\.checkpoint$/;

/** Keep REST caches coherent with the live event stream, batching bursts of events. */
function useEventInvalidation(slug: string, feed: ProjectFeed) {
  const pending = useRef(new Set<string>());
  const timer = useRef<number | null>(null);
  useEffect(() => {
    const flush = () => {
      timer.current = null;
      const scopes = pending.current;
      pending.current = new Set();
      if (scopes.has("status")) queryClient.invalidateQueries({ queryKey: keys.status(slug) });
      if (scopes.has("knowledge")) {
        queryClient.invalidateQueries({ queryKey: keys.snapshot(slug) });
        queryClient.invalidateQueries({ queryKey: keys.knowledge(slug) });
      }
      if (scopes.has("metrics")) queryClient.invalidateQueries({ queryKey: keys.metrics(slug) });
      if (scopes.has("guidance")) queryClient.invalidateQueries({ queryKey: keys.guidance(slug) });
      if (scopes.has("runs")) {
        queryClient.invalidateQueries({ queryKey: keys.runs(slug) });
        queryClient.invalidateQueries({ queryKey: keys.analytics(slug) });
        queryClient.invalidateQueries({ queryKey: keys.workspace(slug) });
        queryClient.invalidateQueries({ queryKey: keys.outputs(slug) });
      }
      if (scopes.has("projects")) queryClient.invalidateQueries({ queryKey: keys.projects });
    };
    const offEvents = feed.onEvents((events: LiminaEvent[]) => {
      for (const event of events) {
        if (event.type.startsWith("runtime.codex") || event.type.startsWith("runtime.claude")) continue;
        pending.current.add("status");
        if (event.type.startsWith("guidance.") || event.type === "project.checkpoint") pending.current.add("guidance");
        if (event.type === "project.metric.updated") pending.current.add("metrics");
        if (KNOWLEDGE_EVENT.test(event.type)) pending.current.add("knowledge");
        if (event.type.startsWith("runtime.turn") || event.type === "runtime.failed") pending.current.add("runs");
        if (event.type.startsWith("project.")) pending.current.add("projects");
      }
      if (pending.current.size && timer.current === null) timer.current = window.setTimeout(flush, 600);
    });
    const offSnapshot = feed.onSnapshot((status: ProjectStatusResponse) => {
      queryClient.setQueryData<ProjectStatusResponse>(keys.status(slug), (current) =>
        current ? { ...status, project: { ...status.project, role: current.project.role } } : status,
      );
    });
    return () => {
      offEvents();
      offSnapshot();
      if (timer.current !== null) window.clearTimeout(timer.current);
    };
  }, [slug, feed]);
}

export function ProjectLayout() {
  const slug = useParams().slug!;
  const feed = useMemo(() => new ProjectFeed(slug), [slug]);
  const status = useProjectStatus(slug);

  useEffect(() => {
    feed.start();
    try {
      localStorage.setItem("limina.lastProject", slug);
    } catch {
      /* ignore */
    }
    return () => feed.close();
  }, [feed, slug]);
  useEventInvalidation(slug, feed);

  if (status.error instanceof ApiError && [403, 404].includes(status.error.status)) {
    return (
      <div className="page page-narrow">
        <Empty
          icon={<WarningAlt size={20} />}
          title={status.error.status === 404 ? "Project not found" : "You don't have access to this project"}
          action={
            <Link to="/projects" className="btn">
              View all projects
            </Link>
          }
        >
          {status.error.message}
        </Empty>
      </div>
    );
  }

  return (
    <ProjectContext.Provider value={{ slug, feed }}>
      <Outlet key={slug} />
    </ProjectContext.Provider>
  );
}
