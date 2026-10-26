import { Add, Edit, Plug, TrashCan, WarningAltFilled } from "@carbon/icons-react";
import { useMutation } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { Link, useSearchParams } from "react-router";
import { ConnectionStatusTag } from "../components/connections/ConnectionStatusTag";
import { useCrumbs } from "../components/shell/Crumbs";
import { Confirm } from "../components/ui/Confirm";
import { Empty, Skeleton } from "../components/ui/Empty";
import { Time } from "../components/ui/Time";
import { useToast } from "../components/ui/Toast";
import {
  AUTH_OPTIONS,
  authLabel,
  canDisconnect,
  connectErrorMessage,
  connectionKeys,
  connectionStatus,
  connectionsApi,
  connectionsChanged,
  connectLabel,
  headerNameProblem,
  projectsPhrase,
  readConnectReturn,
  safeAuthorizationUrl,
  serverUrlProblem,
  takesSecret,
  useConnections,
  type Connection,
  type ConnectionAuth,
  type ConnectionChanges,
} from "../lib/connections";
import { queryClient } from "../lib/queries";

/** Where the provider sign-in comes back to. */
const RETURN_TO = "/connections";

function replaceCached(connection: Connection) {
  queryClient.setQueryData<Connection[]>(connectionKeys.mine, (list) =>
    list?.some((item) => item.id === connection.id)
      ? list.map((item) => (item.id === connection.id ? connection : item))
      : [...(list ?? []), connection],
  );
}

/** Tell the person how a check went (none/bearer/header connect right away, without a redirect). */
function useReportCheck() {
  const toast = useToast();
  return (connection: Connection, title?: string) => {
    if (connection.status === "CONNECTED") {
      toast(title ?? `${connection.name} is connected`, {
        tone: "success",
        body: "Enable it in a project’s Settings → Connections.",
      });
    } else {
      toast(`Couldn’t connect ${connection.name}`, {
        tone: "critical",
        body: connection.last_error ?? connectionStatus(connection.status).label,
      });
    }
  };
}

/**
 * Connect a connection: sign in at the provider (the whole window goes there and comes back to
 * this page), or check a token/header/no-auth connection right away.
 */
function useConnect() {
  const toast = useToast();
  const report = useReportCheck();
  const [redirecting, setRedirecting] = useState<string | null>(null);

  // Coming back with the browser's Back button restores this page from cache: stop the spinner.
  useEffect(() => {
    const onShow = (event: PageTransitionEvent) => event.persisted && setRedirecting(null);
    window.addEventListener("pageshow", onShow);
    return () => window.removeEventListener("pageshow", onShow);
  }, []);

  const mutation = useMutation({
    mutationFn: (connection: Connection) => connectionsApi.connect(connection.id, RETURN_TO),
    onSuccess: (result, connection) => {
      if (result.authorization_url) {
        const url = safeAuthorizationUrl(result.authorization_url);
        if (url) {
          setRedirecting(connection.id);
          window.location.assign(url);
        } else {
          toast(`Couldn’t open the sign-in for ${connection.name}`, {
            tone: "critical",
            body: "The provider returned a sign-in address that isn’t a web page.",
          });
        }
        return;
      }
      replaceCached(result.connection);
      void connectionsChanged();
      report(result.connection);
    },
    onError: (error, connection) => {
      void connectionsChanged();
      toast(`Couldn’t connect ${connection.name}`, { tone: "critical", body: (error as Error).message });
    },
  });

  return {
    start: (connection: Connection) => mutation.mutate(connection),
    busy: (id: string) => redirecting === id || (mutation.isPending && mutation.variables?.id === id),
  };
}

/** Read `?connected=` / `?connection_error=` once after the provider redirects back, then clear them. */
function useConnectReturn() {
  const toast = useToast();
  const [params, setParams] = useSearchParams();
  const handled = useRef<string | null>(null);

  useEffect(() => {
    const outcome = readConnectReturn(params);
    if (!outcome) return;
    const signature = params.toString();
    if (handled.current === signature) return;
    handled.current = signature;
    setParams({}, { replace: true });
    void queryClient
      .fetchQuery({ queryKey: connectionKeys.mine, queryFn: connectionsApi.list, staleTime: 0 })
      .catch(() => undefined)
      .then((list) => {
        const connection = list?.find((item) => item.id === outcome.id);
        if (outcome.kind === "connected") {
          toast(connection ? `${connection.name} is connected` : "Connection connected", {
            tone: "success",
            body: "Enable it in a project’s Settings → Connections.",
          });
        } else {
          toast(connection ? `Couldn’t connect ${connection.name}` : "Couldn’t connect", {
            tone: "critical",
            body: connectErrorMessage(outcome.code, connection?.last_error),
          });
        }
      });
  }, [params, setParams, toast]);
}

/* ─── A connection ──────────────────────────────────────────────────── */

function EditConnection({ connection, onDone }: { connection: Connection; onDone: () => void }) {
  const toast = useToast();
  const report = useReportCheck();
  const [name, setName] = useState(connection.name);
  const [headerName, setHeaderName] = useState(connection.header_name ?? "");
  const [secret, setSecret] = useState("");
  const header = connection.auth === "header";
  const secretLabel = header ? "Header value" : "Token";

  const changes: ConnectionChanges = {};
  if (name.trim() && name.trim() !== connection.name) changes.name = name.trim();
  if (header && headerName.trim() !== (connection.header_name ?? "")) changes.header_name = headerName.trim();
  if (takesSecret(connection.auth) && secret.trim()) changes.secret = secret.trim();
  const headerProblem = header ? headerNameProblem(headerName) : null;
  const ready = Object.keys(changes).length > 0 && !headerProblem && (!header || headerName.trim() !== "");

  const save = useMutation({
    mutationFn: async () => {
      const saved = await connectionsApi.update(connection.id, changes);
      // A new credential is checked right away, so the status reflects it.
      if (changes.secret === undefined && changes.header_name === undefined) return { saved, checked: null };
      try {
        return { saved, checked: (await connectionsApi.connect(connection.id, RETURN_TO)).connection };
      } catch (error) {
        return { saved, checked: null, problem: (error as Error).message };
      }
    },
    onSuccess: ({ saved, checked, problem }) => {
      replaceCached(checked ?? saved);
      void connectionsChanged();
      if (checked) report(checked, `${checked.name} saved and connected`);
      else if (problem) toast(`Saved ${saved.name}, but couldn’t connect`, { tone: "critical", body: problem });
      else toast(`${saved.name} saved`, { tone: "success" });
      onDone();
    },
    onError: (error) => toast("Couldn’t save", { tone: "critical", body: (error as Error).message }),
  });

  return (
    <form
      className="connection-edit form-grid"
      aria-label={`Edit ${connection.name}`}
      onSubmit={(event) => {
        event.preventDefault();
        if (ready) save.mutate();
      }}
    >
      <div className="form-row">
        <label className="field">
          <span className="field-label">Name</span>
          <input className="input" value={name} required onChange={(event) => setName(event.target.value)} autoFocus />
        </label>
        {header && (
          <label className="field">
            <span className="field-label">Header name</span>
            <input
              className="input mono"
              value={headerName}
              required
              spellCheck={false}
              aria-invalid={headerProblem ? true : undefined}
              onChange={(event) => setHeaderName(event.target.value)}
            />
            {headerProblem && (
              <span className="field-hint" data-tone="critical">
                {headerProblem}
              </span>
            )}
          </label>
        )}
      </div>
      {takesSecret(connection.auth) && (
        <label className="field">
          <span className="field-label">Replace {secretLabel.toLowerCase()}</span>
          <input
            className="input mono"
            type="password"
            autoComplete="off"
            spellCheck={false}
            value={secret}
            onChange={(event) => setSecret(event.target.value)}
            placeholder={connection.credential_configured ? "Leave empty to keep the current one" : "Stored encrypted, never shown again"}
          />
          <span className="field-hint">Stored encrypted, never shown again. Limina checks the new value right away.</span>
        </label>
      )}
      <div className="form-actions">
        <button type="button" className="btn btn-ghost btn-sm" onClick={onDone}>
          Cancel
        </button>
        <button type="submit" className="btn btn-primary btn-sm" disabled={!ready || save.isPending}>
          {save.isPending && <span className="spinner" />} Save
        </button>
      </div>
    </form>
  );
}

function ConnectionRow({
  connection,
  connecting,
  onConnect,
  onDisconnect,
  onRemove,
}: {
  connection: Connection;
  connecting: boolean;
  onConnect: () => void;
  onDisconnect: () => void;
  onRemove: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const connected = connection.status === "CONNECTED";
  const expires = connection.expires_at ? new Date(connection.expires_at).getTime() : null;

  return (
    <div className="row-link resource-row connection-row">
      <div className="connection-main">
        <div className="connection-heading">
          <span className="connection-name">{connection.name}</span>
          <ConnectionStatusTag status={connection.status} />
        </div>
        <div className="mono muted resource-value" title={connection.server_url}>
          {connection.server_url}
        </div>
        <div className="connection-meta">
          <span>{authLabel(connection.auth, connection.header_name)}</span>
          {takesSecret(connection.auth) && (
            <span>{connection.credential_configured ? "•••••••• configured" : "No credential stored"}</span>
          )}
          {connection.connected_at && (
            <span>
              Connected <Time iso={connection.connected_at} />
            </span>
          )}
          {expires !== null && (
            <span title="Access is renewed automatically before each turn.">
              {expires > Date.now() ? (
                <>
                  Access renews automatically · expires <Time iso={connection.expires_at} />
                </>
              ) : (
                "Access renews before the next turn"
              )}
            </span>
          )}
          {connection.scopes.length > 0 && (
            <span>
              Scopes <span className="mono">{connection.scopes.join(" ")}</span>
            </span>
          )}
        </div>
        <div className="connection-meta">
          {connection.projects.length ? (
            <span>
              Used by{" "}
              {connection.projects.map((project, index) => (
                <span key={project.slug}>
                  {index > 0 && ", "}
                  <Link className="link" to={`/projects/${project.slug}/settings/connections`}>
                    {project.name}
                  </Link>
                </span>
              ))}
            </span>
          ) : (
            <span>Not enabled in any project</span>
          )}
        </div>
        {connection.last_error && (
          <div className="notice" data-tone={connection.status === "ERROR" ? "critical" : "warning"} role="status">
            <WarningAltFilled size={16} />
            <div>{connection.last_error}</div>
          </div>
        )}
        {editing && <EditConnection connection={connection} onDone={() => setEditing(false)} />}
      </div>
      {!editing && (
        <div className="connection-actions">
          <button
            type="button"
            className={connected ? "btn btn-sm" : "btn btn-primary btn-sm"}
            onClick={onConnect}
            disabled={connecting}
          >
            {connecting && <span className="spinner" />} {connectLabel(connection)}
          </button>
          {canDisconnect(connection) && (
            <button type="button" className="btn btn-sm" onClick={onDisconnect}>
              Disconnect
            </button>
          )}
          <button
            type="button"
            className="btn btn-ghost btn-icon btn-sm"
            aria-label={`Edit ${connection.name}`}
            title="Edit"
            onClick={() => setEditing(true)}
          >
            <Edit size={16} />
          </button>
          <button
            type="button"
            className="btn btn-ghost btn-icon btn-sm"
            aria-label={`Remove ${connection.name}`}
            title="Remove"
            onClick={onRemove}
          >
            <TrashCan size={16} />
          </button>
        </div>
      )}
    </div>
  );
}

/* ─── Add ───────────────────────────────────────────────────────────── */

const BLANK = { name: "", server_url: "", header_name: "", secret: "" };

function AddConnection({ onAdded }: { onAdded: (connection: Connection) => void }) {
  const toast = useToast();
  const [auth, setAuth] = useState<ConnectionAuth>("oauth");
  const [form, setForm] = useState(BLANK);
  const urlProblem = serverUrlProblem(form.server_url);
  const header = auth === "header";
  const headerProblem = header ? headerNameProblem(form.header_name) : null;
  const ready =
    form.name.trim() !== "" &&
    form.server_url.trim() !== "" &&
    !urlProblem &&
    (!header || (form.header_name.trim() !== "" && !headerProblem)) &&
    (!takesSecret(auth) || form.secret.trim() !== "");

  const add = useMutation({
    mutationFn: () =>
      connectionsApi.create({
        name: form.name.trim(),
        server_url: form.server_url.trim(),
        auth,
        ...(header ? { header_name: form.header_name.trim() } : {}),
        ...(takesSecret(auth) ? { secret: form.secret.trim() } : {}),
      }),
    onSuccess: (created) => {
      replaceCached(created);
      void connectionsChanged();
      setForm(BLANK);
      onAdded(created);
    },
    onError: (error) => toast("Couldn’t add the connection", { tone: "critical", body: (error as Error).message }),
  });
  const set = (name: keyof typeof BLANK) => (event: React.ChangeEvent<HTMLInputElement>) =>
    setForm((current) => ({ ...current, [name]: event.target.value }));

  return (
    <form
      className="panel"
      onSubmit={(event) => {
        event.preventDefault();
        if (ready) add.mutate();
      }}
    >
      <div className="panel-header">
        <span className="panel-title">Add connection</span>
      </div>
      <div className="panel-body form-grid" style={{ gap: "var(--space-4)" }}>
        <div className="form-row">
          <label className="field">
            <span className="field-label">Name</span>
            <input className="input" value={form.name} required onChange={set("name")} placeholder="Team notes" />
          </label>
          <label className="field">
            <span className="field-label">Server URL</span>
            <input
              className="input mono"
              type="url"
              inputMode="url"
              spellCheck={false}
              value={form.server_url}
              required
              aria-invalid={urlProblem ? true : undefined}
              onChange={set("server_url")}
              placeholder="https://mcp.example.com/mcp"
            />
            {urlProblem && (
              <span className="field-hint" data-tone="critical">
                {urlProblem}
              </span>
            )}
          </label>
        </div>
        <div className="field">
          <span className="field-label" id="connection-auth-label">
            Authorization
          </span>
          <div className="segmented" role="group" aria-labelledby="connection-auth-label" style={{ justifySelf: "start" }}>
            {AUTH_OPTIONS.map((option) => (
              <button key={option.value} type="button" aria-pressed={auth === option.value} onClick={() => setAuth(option.value)}>
                {option.label}
              </button>
            ))}
          </div>
          <span className="field-hint">{AUTH_OPTIONS.find((option) => option.value === auth)?.hint}</span>
        </div>
        {takesSecret(auth) && (
          <div className="form-row">
            {header && (
              <label className="field">
                <span className="field-label">Header name</span>
                <input
                  className="input mono"
                  value={form.header_name}
                  required
                  spellCheck={false}
                  aria-invalid={headerProblem ? true : undefined}
                  onChange={set("header_name")}
                  placeholder="X-API-Key"
                />
                {headerProblem && (
                  <span className="field-hint" data-tone="critical">
                    {headerProblem}
                  </span>
                )}
              </label>
            )}
            <label className="field">
              <span className="field-label">{header ? "Header value" : "Token"}</span>
              <input
                className="input mono"
                type="password"
                autoComplete="off"
                spellCheck={false}
                value={form.secret}
                required
                onChange={set("secret")}
                placeholder="Stored encrypted, never shown again"
              />
            </label>
          </div>
        )}
      </div>
      <div className="panel-footer">
        <span>
          {auth === "oauth"
            ? "After adding it, you’re taken to the provider to sign in."
            : "Limina checks the server as soon as you add it."}
        </span>
        <button className="btn btn-primary btn-sm" type="submit" disabled={!ready || add.isPending}>
          {add.isPending ? <span className="spinner" /> : <Add size={16} />} Add connection
        </button>
      </div>
    </form>
  );
}

/* ─── Page ──────────────────────────────────────────────────────────── */

type Pending = { kind: "disconnect" | "remove"; connection: Connection };

export function Connections() {
  useCrumbs([{ label: "Connections" }]);
  useConnectReturn();
  const connections = useConnections();
  const toast = useToast();
  const connect = useConnect();
  const [pending, setPending] = useState<Pending | null>(null);

  const disconnect = useMutation({
    mutationFn: (connection: Connection) => connectionsApi.disconnect(connection.id),
    onSuccess: (saved) => {
      setPending(null);
      replaceCached(saved);
      void connectionsChanged();
      toast(`${saved.name} disconnected`, { body: "Its credentials were cleared and no project uses it." });
    },
    onError: (error) => toast("Couldn’t disconnect", { tone: "critical", body: (error as Error).message }),
  });
  const remove = useMutation({
    mutationFn: (connection: Connection) => connectionsApi.remove(connection.id),
    onSuccess: (removed) => {
      setPending(null);
      queryClient.setQueryData<Connection[]>(connectionKeys.mine, (list) => list?.filter((item) => item.id !== removed.id));
      void connectionsChanged();
      toast(`${removed.name} removed`);
    },
    onError: (error) => toast("Couldn’t remove", { tone: "critical", body: (error as Error).message }),
  });

  const items = connections.data ?? [];
  const target = pending?.connection;
  const used = target && target.projects.length > 0 ? projectsPhrase(target.projects) : null;

  return (
    <div className="page page-narrow">
      <div className="page-header">
        <div>
          <h1 className="page-title">Connections</h1>
          <p className="page-subtitle">
            MCP servers on your account that projects can use to reach your data. Connecting asks for your
            permission at the provider. A project uses a connection only after you enable it in the project’s
            Settings → Connections. Credentials are encrypted and never shown again.
          </p>
        </div>
      </div>

      {connections.isPending ? (
        <Skeleton height={160} />
      ) : connections.error ? (
        <div className="notice" data-tone="critical" role="alert">
          <WarningAltFilled size={16} />
          <div style={{ flex: 1 }}>
            <div className="notice-title">Couldn’t load your connections</div>
            <div className="notice-body">{connections.error.message}</div>
          </div>
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => connections.refetch()}>
            Try again
          </button>
        </div>
      ) : items.length ? (
        <div className="panel" aria-label="Your connections">
          {items.map((connection) => (
            <ConnectionRow
              key={connection.id}
              connection={connection}
              connecting={connect.busy(connection.id)}
              onConnect={() => connect.start(connection)}
              onDisconnect={() => setPending({ kind: "disconnect", connection })}
              onRemove={() => setPending({ kind: "remove", connection })}
            />
          ))}
        </div>
      ) : (
        <Empty icon={<Plug size={20} />} title="No connections yet">
          Add a remote MCP server below. Once it’s connected, enable it in a project so that project can use it.
        </Empty>
      )}

      <section className="section">
        <AddConnection onAdded={connect.start} />
      </section>

      <Confirm
        open={pending !== null}
        onOpenChange={(open) => !open && setPending(null)}
        danger
        title={pending?.kind === "remove" ? `Remove ${target?.name}?` : `Disconnect ${target?.name}?`}
        body={
          pending?.kind === "remove"
            ? `The connection and its credentials are deleted${used ? `, and it’s removed from ${used}` : ""}. This can’t be undone.`
            : `Its credentials are cleared${used ? ` and it’s removed from ${used}` : ""}. You can connect it again later and enable it in projects again.`
        }
        action={pending?.kind === "remove" ? "Remove" : "Disconnect"}
        pending={disconnect.isPending || remove.isPending}
        onConfirm={() => {
          if (!pending) return;
          if (pending.kind === "remove") remove.mutate(pending.connection);
          else disconnect.mutate(pending.connection);
        }}
      />
    </div>
  );
}
