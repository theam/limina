import { QueryClient, useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { ApiError, api } from "./api";
import type { Engine } from "./types";
import { buildVault } from "./vault";
import type { KnowledgeFilter } from "./knowledge";

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 15_000,
      refetchOnWindowFocus: true,
      retry: (count, error) =>
        !(error instanceof ApiError && [401, 403, 404].includes(error.status)) && count < 2,
    },
  },
});

export const keys = {
  me: ["me"] as const,
  authConfig: ["auth-config"] as const,
  engines: ["engines"] as const,
  projects: ["projects"] as const,
  templates: ["templates"] as const,
  status: (slug: string) => ["project", slug, "status"] as const,
  preflight: (slug: string) => ["project", slug, "preflight"] as const,
  snapshot: (slug: string) => ["project", slug, "snapshot"] as const,
  knowledge: (slug: string) => ["project", slug, "knowledge"] as const,
  runs: (slug: string) => ["project", slug, "runs"] as const,
  metrics: (slug: string) => ["project", slug, "metrics"] as const,
  workspace: (slug: string) => ["project", slug, "workspace"] as const,
  guidance: (slug: string) => ["project", slug, "guidance"] as const,
  outputs: (slug: string) => ["project", slug, "outputs"] as const,
  analytics: (slug: string) => ["project", slug, "analytics"] as const,
  resources: (slug: string) => ["project", slug, "resources"] as const,
  sources: (slug: string) => ["project", slug, "sources"] as const,
  members: (slug: string) => ["project", slug, "members"] as const,
  engineAccounts: (slug: string) => ["project", slug, "engine-accounts"] as const,
};

export const useAuthConfig = () =>
  useQuery({ queryKey: keys.authConfig, queryFn: api.authConfig, staleTime: Infinity });
export const useMe = () => useQuery({ queryKey: keys.me, queryFn: api.me, staleTime: 5 * 60_000 });
export const useEngines = () => useQuery({ queryKey: keys.engines, queryFn: api.engines, staleTime: 60_000 });
export const useModelCatalog = (engine: Engine, slug?: string) =>
  useQuery({ queryKey: ["models", engine, slug ?? null], queryFn: () => api.models(engine, slug), staleTime: 60_000, refetchOnMount: "always", refetchInterval: 60_000, retry: false });
export const useProjects = () =>
  useQuery({ queryKey: keys.projects, queryFn: () => api.projects(), refetchInterval: 30_000 });
export const useTemplates = () =>
  useQuery({ queryKey: keys.templates, queryFn: api.templates, staleTime: Infinity });

export const useProjectStatus = (slug: string) =>
  useQuery({ queryKey: keys.status(slug), queryFn: () => api.status(slug), refetchInterval: 30_000 });
export const usePreflight = (slug: string) =>
  useQuery({ queryKey: keys.preflight(slug), queryFn: () => api.preflight(slug) });
export const useKnowledgeList = (slug: string, filters: KnowledgeFilter = {}, enabled = true) =>
  useQuery({ queryKey: [...keys.knowledge(slug), filters], queryFn: () => api.knowledge(slug, filters), enabled });

export function useKnowledgeDocument(slug: string, id: string) {
  const query = useQuery({ queryKey: [...keys.knowledge(slug), "document", id], queryFn: () => api.knowledgeDocument(slug, id) });
  const vault = useMemo(() => query.data ? buildVault(query.data.files) : undefined, [query.data]);
  return { ...query, vault };
}
export const useRuns = (slug: string, limit = 200) => useQuery({ queryKey: [...keys.runs(slug), limit], queryFn: () => api.runs(slug, limit) });
export const useMetrics = (slug: string) => useQuery({ queryKey: keys.metrics(slug), queryFn: () => api.metrics(slug) });
export const useWorkspace = (slug: string) => useQuery({ queryKey: keys.workspace(slug), queryFn: () => api.workspace(slug) });
export const useGuidance = (slug: string) => useQuery({ queryKey: keys.guidance(slug), queryFn: () => api.guidance(slug) });
export const useAnalytics = (slug: string) =>
  useQuery({ queryKey: keys.analytics(slug), queryFn: () => api.analytics(slug) });
export const useResources = (slug: string) =>
  useQuery({ queryKey: keys.resources(slug), queryFn: () => api.resources(slug) });
export const useSources = (slug: string) =>
  useQuery({ queryKey: keys.sources(slug), queryFn: () => api.sources(slug) });
export const useEngineAccounts = (slug: string) =>
  useQuery({ queryKey: keys.engineAccounts(slug), queryFn: () => api.engineAccounts(slug) });
export const useMembers = (slug: string) =>
  useQuery({ queryKey: keys.members(slug), queryFn: () => api.members(slug) });

export function useVault(slug: string) {
  const query = useQuery({ queryKey: keys.snapshot(slug), queryFn: () => api.snapshot(slug) });
  const vault = useMemo(() => (query.data ? buildVault(query.data.files) : undefined), [query.data]);
  return { ...query, vault };
}
