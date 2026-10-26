import type { Metric, MetricInput, WorkspaceAssessment, Revision, OutputPage, OutputPreview } from "./types";
import type { KnowledgeFilter } from "./knowledge";
import type {
  Analytics,
  AuthMode,
  Artifact,
  CodexDeviceLogin,
  ProjectEngineAccount,
  EngineInfo,
  EventPage,
  GuidanceKind,
  Guidance,
  GuidanceReceipt,
  KickoffTemplate,
  ModelCatalog,
  KnowledgePage,
  LifecycleAction,
  Me,
  Member,
  Page,
  Preflight,
  Project,
  ProjectStatusResponse,
  Resource,
  Role,
  Run,
  Source,
  Engine,
} from "./types";

/* ─── Session ──────────────────────────────────────────────────────────── */

const TOKEN_KEY = "limina.token";
const ACTOR_KEY = "limina.actor";

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string | null): void {
  try {
    if (value) localStorage.setItem(key, value);
    else localStorage.removeItem(key);
  } catch {
    /* storage unavailable: the session lasts for this page only */
  }
}

let token = read(TOKEN_KEY);
let actor = read(ACTOR_KEY);

export const session = {
  get token() {
    return token;
  },
  get actor() {
    return actor;
  },
  set(next: { token: string | null; actor?: string | null }) {
    token = next.token;
    write(TOKEN_KEY, token);
    if (next.actor !== undefined) {
      actor = next.actor?.trim() || null;
      write(ACTOR_KEY, actor);
    }
  },
  clear() {
    token = null;
    write(TOKEN_KEY, null);
  },
};

/** Accept `#token=…` links (the fragment never reaches the server) and strip them from history. */
export function consumeTokenFragment(): void {
  const match = window.location.hash.match(/(?:^#|&)token=([^&]+)/);
  if (!match) return;
  session.set({ token: decodeURIComponent(match[1]) });
  history.replaceState(null, "", window.location.pathname + window.location.search);
}

/* ─── Transport ────────────────────────────────────────────────────────── */

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
    readonly details: Record<string, unknown> = {},
  ) {
    super(message);
  }
}

const unauthorizedListeners = new Set<() => void>();
export function onUnauthorized(listener: () => void): () => void {
  unauthorizedListeners.add(listener);
  return () => unauthorizedListeners.delete(listener);
}

function commandId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) return crypto.randomUUID();
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

interface RequestOptions {
  method?: "GET" | "POST" | "PUT" | "PATCH" | "DELETE";
  body?: unknown;
  command?: boolean;
  signal?: AbortSignal;
  blob?: boolean;
}

export async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const headers: Record<string, string> = { Accept: "application/json" };
  if (token) headers.Authorization = `Bearer ${token}`;
  if (actor) headers["X-Limina-Actor"] = actor;
  if (options.command) headers["Idempotency-Key"] = commandId();
  let body: BodyInit | undefined;
  if (options.body instanceof FormData) {
    body = options.body;
  } else if (options.body !== undefined) {
    headers["Content-Type"] = "application/json";
    body = JSON.stringify(options.body);
  }
  const response = await fetch(path, {
    method: options.method ?? "GET",
    headers,
    body,
    signal: options.signal,
  });
  if (response.ok) {
    if (options.blob) return await response.blob() as T;
    if (response.status === 204) return undefined as T;
    return (await response.json()) as T;
  }
  let code = "http_error";
  let message = `Request failed (${response.status}).`;
  let details: Record<string, unknown> = {};
  try {
    const payload = await response.json();
    if (payload?.error) {
      code = payload.error.code ?? code;
      message = payload.error.message ?? message;
      details = payload.error.details ?? {};
    } else if (typeof payload?.detail === "string") {
      message = payload.detail;
    } else if (Array.isArray(payload?.detail) && payload.detail[0]?.msg) {
      message = payload.detail[0].msg;
    }
  } catch {
    /* non-JSON error body */
  }
  if (response.status === 401) unauthorizedListeners.forEach((listener) => listener());
  throw new ApiError(response.status, code, message, details);
}

const p = (slug: string) => `/v1/projects/${encodeURIComponent(slug)}`;

/* ─── Endpoints ────────────────────────────────────────────────────────── */

/** Browser sessions (WorkOS): sign out server-side, then end the provider session. */
export async function signOutOfSession(): Promise<string> {
  const response = await fetch("/auth/logout", { method: "POST", credentials: "same-origin" });
  const payload = response.ok ? ((await response.json()) as { redirect?: string }) : {};
  return payload.redirect ?? "/signin?signed_out=1";
}

export const api = {
  authConfig: () => request<{ mode: AuthMode }>("/auth/config"),
  me: () => request<Me>("/v1/me"),
  engines: () => request<EngineInfo[]>("/v1/runtime/engines"),
  models: (engine: Engine, slug?: string) => request<ModelCatalog>(`/v1/runtime/engines/${engine}/models${slug ? `?project=${encodeURIComponent(slug)}` : ""}`),

  projects: (includeArchived = false) =>
    request<Page<Project>>(`/v1/projects?limit=200&include_archived=${includeArchived}`),
  templates: () => request<KickoffTemplate[]>("/v1/project-templates"),
  createProject: (body: {
    slug: string;
    name: string;
    objective: string;
    success_criteria: string;
    context: string;
    runtime: Engine;
    model?: string;
    reasoning_effort?: string;
  }) => request<Project>("/v1/projects", { method: "POST", body, command: true }),
  updateProject: (
    slug: string,
    body: Partial<Pick<Project, "name" | "mission" | "success_criteria" | "context" | "runtime" | "model" | "reasoning_effort">>,
  ) => request<Project>(p(slug), { method: "PATCH", body }),
  status: (slug: string) => request<ProjectStatusResponse>(`${p(slug)}/status`),
  guidance: (slug: string) => request<Page<Guidance>>(`${p(slug)}/guidance?limit=50`),
  workspace: (slug: string) => request<WorkspaceAssessment | null>(`${p(slug)}/workspace`),
  revisions: (slug: string, id: string, before = 2147483647) => request<{ items: Revision[]; next_before: number | null }>(`${p(slug)}/knowledge/${encodeURIComponent(id)}/history?before=${before}`),
  outputs: (slug: string, offset = 0) => request<OutputPage>(`${p(slug)}/outputs?offset=${offset}`),
  outputPreview: (slug: string, name: string) => request<OutputPreview>(`${p(slug)}/outputs/${encodeURIComponent(name)}`),
  outputDownload: (slug: string, name: string) => request<Blob>(`${p(slug)}/outputs/${encodeURIComponent(name)}/download`, { blob: true }),
  preflight: (slug: string) => request<Preflight>(`${p(slug)}/preflight`),
  lifecycle: (slug: string, action: LifecycleAction) =>
    request<Project>(`${p(slug)}/actions/${action}`, { method: "POST", command: true }),

  steer: (slug: string, body: string, kind: GuidanceKind) =>
    request<GuidanceReceipt>(`${p(slug)}/steering`, {
      method: "POST",
      body: { body, kind },
      command: true,
    }),
  events: (slug: string, query: { before?: number; after?: number; limit?: number }) => {
    const params = new URLSearchParams();
    if (query.before !== undefined) params.set("before", String(query.before));
    if (query.after !== undefined) params.set("after", String(query.after));
    params.set("limit", String(query.limit ?? 300));
    return request<EventPage>(`${p(slug)}/events?${params}`);
  },
  liveTicket: (slug: string) =>
    request<{ ticket: string; expires_at: string }>(`${p(slug)}/live-ticket`, { method: "POST" }),

  snapshot: (slug: string) => request<{ files: Record<string, string> }>(`${p(slug)}/snapshot`),
  knowledge: (slug: string, query: KnowledgeFilter = {}) => {
    const params = new URLSearchParams({ limit: String(query.limit ?? 200) });
    for (const key of ["query", "kind", "status", "tag", "parent", "cursor"] as const) {
      if (query[key]) params.set(key, query[key]);
    }
    return request<KnowledgePage>(`${p(slug)}/knowledge?${params}`);
  },
  knowledgeDocument: (slug: string, id: string) =>
    request<{ files: Record<string, string> }>(`${p(slug)}/knowledge/${encodeURIComponent(id)}/document`),
  artifact: (slug: string, id: string) =>
    request<Artifact>(`${p(slug)}/knowledge/${encodeURIComponent(id)}`),

  metrics: (slug: string) => request<Metric[]>(`${p(slug)}/metrics`),
  setMetric: (slug: string, id: string, metric: MetricInput, expected_version: number) =>
    request<Metric>(`${p(slug)}/metrics/${encodeURIComponent(id)}`, { method: "PUT", command: true, body: { metric, expected_version } }),
  runs: (slug: string, limit = 200) => request<Page<Run>>(`${p(slug)}/runs?limit=${limit}`),
  analytics: (slug: string, days = 30) =>
    request<Analytics>(`${p(slug)}/analytics?days=${days}`),

  resources: (slug: string) => request<Resource[]>(`${p(slug)}/resources`),
  setVariable: (slug: string, name: string, value: string) =>
    request<Resource>(`${p(slug)}/resources/variables/${encodeURIComponent(name)}`, {
      method: "PUT",
      body: { value },
      command: true,
    }),
  setSecret: (slug: string, name: string, value: string) =>
    request<Resource>(`${p(slug)}/resources/secrets/${encodeURIComponent(name)}`, {
      method: "PUT",
      body: { value },
      command: true,
    }),
  removeResource: (slug: string, name: string) =>
    request<Resource>(`${p(slug)}/resources/${encodeURIComponent(name)}`, {
      method: "DELETE",
      command: true,
    }),

  sources: (slug: string) => request<Source[]>(`${p(slug)}/sources`),
  setSource: (slug: string, body: { name: string; type: "URL" | "CONNECTOR"; uri: string }) =>
    request<Source>(`${p(slug)}/sources`, { method: "PUT", body }),
  uploadSource: (slug: string, name: string, file: File) => {
    const form = new FormData();
    form.set("name", name);
    form.set("file", file);
    return request<Source>(`${p(slug)}/sources/upload`, { method: "POST", body: form });
  },
  removeSource: (slug: string, id: string) =>
    request<Source>(`${p(slug)}/sources/${encodeURIComponent(id)}`, { method: "DELETE" }),

  members: (slug: string) => request<Member[]>(`${p(slug)}/members`),
  setMember: (
    slug: string,
    body: { subject: string; display_name: string; email: string | null; role: Role },
  ) => request<Member>(`${p(slug)}/members`, { method: "PUT", body }),
  removeMember: (slug: string, subject: string) =>
    request<Member>(`${p(slug)}/members?subject=${encodeURIComponent(subject)}`, {
      method: "DELETE",
    }),

  engineAccounts: (slug: string) =>
    request<ProjectEngineAccount[]>(`${p(slug)}/engine-accounts`),
  connectClaudeAccount: (slug: string, token: string) =>
    request<ProjectEngineAccount>(`${p(slug)}/engine-accounts/claude-code`, {
      method: "PUT",
      body: { token },
    }),
  startClaudeAccountLogin: (slug: string) =>
    request<CodexDeviceLogin>(`${p(slug)}/engine-accounts/claude-code/login`, {
      method: "POST",
      command: true,
    }),
  submitClaudeAccountCode: (slug: string, id: string, code: string) =>
    request<CodexDeviceLogin>(`${p(slug)}/engine-accounts/claude-code/login/${encodeURIComponent(id)}`, {
      method: "PUT",
      body: { code },
    }),
  cancelClaudeAccountLogin: (slug: string, id: string) =>
    request<CodexDeviceLogin>(`${p(slug)}/engine-accounts/claude-code/login/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }),
  startCodexAccountLogin: (slug: string) =>
    request<CodexDeviceLogin>(`${p(slug)}/engine-accounts/codex/login`, {
      method: "POST",
      command: true,
    }),
  codexAccountLogin: (slug: string, id: string) =>
    request<CodexDeviceLogin>(`${p(slug)}/engine-accounts/codex/login/${encodeURIComponent(id)}`),
  cancelCodexAccountLogin: (slug: string, id: string) =>
    request<CodexDeviceLogin>(`${p(slug)}/engine-accounts/codex/login/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }),
  disconnectEngineAccount: (slug: string, engine: Engine) =>
    request<ProjectEngineAccount>(`${p(slug)}/engine-accounts/${engine}`, { method: "DELETE" }),

};
