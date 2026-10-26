import { useQuery } from "@tanstack/react-query";
import { request } from "./api";
import type { Tone } from "./format";
import { queryClient } from "./queries";

/* ─── Types ────────────────────────────────────────────────────────────── */

/** oauth = sign in at the provider; none; bearer = pasted token; header = pasted header value. */
export type ConnectionAuth = "oauth" | "none" | "bearer" | "header";
export type ConnectionStatus = "NOT_CONNECTED" | "CONNECTED" | "NEEDS_AUTHORIZATION" | "ERROR";
export type InactiveReason = "not_connected" | "owner_left";
/** Why a sign-in at the provider didn't complete (`?connection_error=` on the way back). */
export type ConnectErrorCode = "denied" | "expired" | "browser" | "failed";

/** A remote MCP server on the caller's own account. Credentials are write-only. */
export interface Connection {
  id: string;
  name: string;
  /** Streamable HTTP MCP server URL. */
  server_url: string;
  auth: ConnectionAuth;
  /** For auth "header". */
  header_name: string | null;
  status: ConnectionStatus;
  /** Human-readable, safe to show. */
  last_error: string | null;
  /** A token, header value or OAuth grant is stored (never shown). */
  credential_configured: boolean;
  /** Granted OAuth scopes. */
  scopes: string[];
  /** OAuth access token expiry (refreshed automatically before turns). */
  expires_at: string | null;
  connected_at: string | null;
  created_at: string;
  updated_at: string;
  /** Projects where this connection is enabled. */
  projects: { slug: string; name: string }[];
}

export interface ConnectResult {
  connection: Connection;
  /** Navigate the whole window here to sign in at the provider; null when the check already ran. */
  authorization_url: string | null;
}

/** A connection enabled in a project, as every member sees it (never credentials). */
export interface ProjectConnection {
  connection_id: string;
  name: string;
  server_url: string;
  /** The MCP server name the engine sees, e.g. "team_notes". */
  server_key: string;
  /** The owner's display name. */
  owner: string;
  owned_by_you: boolean;
  status: ConnectionStatus;
  /** Will be used by the next turn. */
  active: boolean;
  inactive_reason: InactiveReason | null;
  enabled_by: string;
  enabled_at: string;
}

export interface AvailableConnection {
  connection_id: string;
  name: string;
  server_url: string;
  status: ConnectionStatus;
}

export interface ProjectConnections {
  /** Enabled in this project (every member sees this). */
  connections: ProjectConnection[];
  /** The caller's own connections not enabled here (empty unless can_enable). */
  available: AvailableConnection[];
  /** The caller is an owner of the project. */
  can_enable: boolean;
}

export interface NewConnection {
  name: string;
  server_url: string;
  auth?: ConnectionAuth;
  header_name?: string;
  /** Required for bearer/header, refused for oauth/none. */
  secret?: string;
}

/** The server URL and auth kind can't change: remove the connection and add it again. */
export interface ConnectionChanges {
  name?: string;
  header_name?: string;
  secret?: string;
}

/* ─── API ──────────────────────────────────────────────────────────────── */

const one = (id: string) => `/v1/connections/${encodeURIComponent(id)}`;
const inProject = (slug: string) => `/v1/projects/${encodeURIComponent(slug)}/connections`;

export const connectionsApi = {
  list: () => request<Connection[]>("/v1/connections"),
  get: (id: string) => request<Connection>(one(id)),
  create: (body: NewConnection) =>
    request<Connection>("/v1/connections", { method: "POST", body, command: true }),
  update: (id: string, body: ConnectionChanges) => request<Connection>(one(id), { method: "PATCH", body }),
  /** Removes the connection, and with it from every project. */
  remove: (id: string) => request<Connection>(one(id), { method: "DELETE" }),
  /** `returnTo` is a relative console path the provider sign-in comes back to (default `/connections`). */
  connect: (id: string, returnTo?: string) =>
    request<ConnectResult>(`${one(id)}/connect`, {
      method: "POST",
      body: returnTo ? { return_to: returnTo } : {},
    }),
  /** Clears the credentials and removes the connection from every project. */
  disconnect: (id: string) => request<Connection>(`${one(id)}/disconnect`, { method: "POST" }),

  forProject: (slug: string) => request<ProjectConnections>(inProject(slug)),
  enable: (slug: string, id: string) =>
    request<ProjectConnection>(`${inProject(slug)}/${encodeURIComponent(id)}`, { method: "PUT" }),
  disable: (slug: string, id: string) =>
    request<ProjectConnection>(`${inProject(slug)}/${encodeURIComponent(id)}`, { method: "DELETE" }),
};

/* ─── Queries ──────────────────────────────────────────────────────────── */

export const connectionKeys = {
  mine: ["connections"] as const,
  project: (slug: string) => ["project", slug, "connections"] as const,
};

export const useConnections = () => useQuery({ queryKey: connectionKeys.mine, queryFn: connectionsApi.list });
export const useProjectConnections = (slug: string) =>
  useQuery({ queryKey: connectionKeys.project(slug), queryFn: () => connectionsApi.forProject(slug) });

/**
 * Refresh your connections and every project's view of them (and its preflight): disconnecting or
 * removing a connection takes it out of every project.
 */
export function connectionsChanged(): Promise<void> {
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: connectionKeys.mine }),
    queryClient.invalidateQueries({
      predicate: ({ queryKey }) =>
        queryKey[0] === "project" && (queryKey[2] === "connections" || queryKey[2] === "preflight"),
    }),
  ]).then(() => undefined);
}

/* ─── Presentation ─────────────────────────────────────────────────────── */

export const AUTH_OPTIONS: { value: ConnectionAuth; label: string; hint: string }[] = [
  { value: "oauth", label: "Sign in", hint: "You sign in at the provider and choose what Limina may access." },
  { value: "none", label: "No sign-in", hint: "For servers that need no authorization." },
  { value: "bearer", label: "Token", hint: "Sent as Authorization: Bearer <token>." },
  { value: "header", label: "Header", hint: "Sent in the header you name, for servers that use an API key header." },
];

/** True when the connection stores a pasted secret (a token or a header value). */
export const takesSecret = (auth: ConnectionAuth) => auth === "bearer" || auth === "header";

/** How a connection authorizes, for display. */
export function authLabel(auth: ConnectionAuth, headerName?: string | null): string {
  switch (auth) {
    case "oauth":
      return "Sign-in at the provider";
    case "none":
      return "No sign-in";
    case "bearer":
      return "Bearer token";
    case "header":
      return headerName ? `Header ${headerName}` : "Header";
    default:
      return String(auth);
  }
}

export function connectionStatus(status: ConnectionStatus): { label: string; tone: Tone } {
  switch (status) {
    case "CONNECTED":
      return { label: "Connected", tone: "success" };
    case "NEEDS_AUTHORIZATION":
      return { label: "Needs authorization", tone: "warning" };
    case "ERROR":
      return { label: "Error", tone: "critical" };
    case "NOT_CONNECTED":
      return { label: "Not connected", tone: "neutral" };
    default:
      return { label: String(status), tone: "neutral" };
  }
}

/** "Connect" the first time, "Reconnect" once it has been connected or tried. */
export function connectLabel(connection: Pick<Connection, "status" | "credential_configured" | "connected_at">): string {
  return connection.status === "NOT_CONNECTED" && !connection.credential_configured && !connection.connected_at
    ? "Connect"
    : "Reconnect";
}

/** Whether there is anything to disconnect: stored credentials, or a live connection. */
export const canDisconnect = (connection: Pick<Connection, "status" | "credential_configured">) =>
  connection.credential_configured || connection.status !== "NOT_CONNECTED";

/** Why an enabled connection won't be used by the next turn. */
export function inactiveReasonLabel(
  reason: InactiveReason | null,
  ownedByYou = false,
): string | null {
  switch (reason) {
    case "not_connected":
      return ownedByYou ? "Not connected — reconnect it to use it here" : "Not connected — its owner must reconnect it";
    case "owner_left":
      return "Its owner is no longer a member";
    default:
      return null;
  }
}

/** Whether the next turn uses an enabled connection, and why not. */
export function projectConnectionState(
  item: Pick<ProjectConnection, "active" | "inactive_reason" | "owned_by_you" | "status">,
): { label: string; tone: Tone; detail: string | null } {
  if (item.active) return { label: "Active", tone: "success", detail: null };
  return {
    label: "Inactive",
    tone: "warning",
    detail: inactiveReasonLabel(item.inactive_reason, item.owned_by_you) ?? connectionStatus(item.status).label,
  };
}

/** The outcome of a provider sign-in, read from the URL the runtime redirects back to. */
export type ConnectReturn =
  | { kind: "connected"; id: string }
  | { kind: "error"; id: string | null; code: ConnectErrorCode | string };

export function readConnectReturn(params: URLSearchParams): ConnectReturn | null {
  const connected = params.get("connected");
  if (connected) return { kind: "connected", id: connected };
  const code = params.get("connection_error");
  if (code) return { kind: "error", id: params.get("connection"), code };
  return null;
}

/**
 * A clear message for each way a provider sign-in can fail. For `failed`, the connection's
 * `last_error` (when known) says what the provider refused.
 */
export function connectErrorMessage(code: ConnectErrorCode | string, lastError?: string | null): string {
  switch (code) {
    case "denied":
      return "Permission wasn’t granted at the provider. Connect again to choose what Limina may access.";
    case "expired":
      return "The sign-in took too long or its link was already used. Connect again to start over.";
    case "browser":
      return "Finish the sign-in in the same browser that started it. Connect again from this browser.";
    case "failed":
      return lastError?.trim() || "The provider refused the sign-in. Try connecting again.";
    default:
      return "The sign-in didn’t complete. Try connecting again.";
  }
}

/** "Alpha", "Alpha and Beta", "3 projects": the projects a change affects, for confirmations. */
export function projectsPhrase(projects: { name: string }[]): string {
  if (projects.length === 1) return projects[0].name;
  if (projects.length === 2) return `${projects[0].name} and ${projects[1].name}`;
  return `${projects.length} projects`;
}

/** An HTTP header name: one token, no spaces or separators. Null when fine (or still empty). */
export function headerNameProblem(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  return /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(trimmed) ? null : "Use a header name like X-API-Key, without spaces.";
}

const LOOPBACK = /^(localhost|127(?:\.\d{1,3}){3}|\[::1\])$/i;

/** Mirrors the server's rule: https, or plain http only for localhost/loopback. Null when fine. */
export function serverUrlProblem(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  let url: URL;
  try {
    url = new URL(trimmed);
  } catch {
    return "Enter the full server URL, like https://mcp.example.com/mcp.";
  }
  if (url.username || url.password) return "Don’t put credentials in the URL; use a token or header instead.";
  if (url.protocol === "https:") return null;
  if (url.protocol === "http:" && (LOOPBACK.test(url.hostname) || url.hostname.endsWith(".localhost"))) return null;
  if (url.protocol === "http:") return "Use https:// (plain http works only for localhost).";
  return "Use an https:// URL.";
}

/** Only follow an http(s) authorization URL; anything else (e.g. `javascript:`) is refused. */
export function safeAuthorizationUrl(value: string | null | undefined): string | null {
  if (!value) return null;
  try {
    const url = new URL(value, window.location.origin);
    return url.protocol === "https:" || url.protocol === "http:" ? url.href : null;
  } catch {
    return null;
  }
}
