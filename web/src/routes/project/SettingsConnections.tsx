import { ArrowRight, Plug, WarningAltFilled } from "@carbon/icons-react";
import { useMutation } from "@tanstack/react-query";
import { Link } from "react-router";
import { ConnectionStatusTag } from "../../components/connections/ConnectionStatusTag";
import { useCrumbs } from "../../components/shell/Crumbs";
import { Empty, Skeleton } from "../../components/ui/Empty";
import { Tag } from "../../components/ui/Status";
import { Time } from "../../components/ui/Time";
import { useToast } from "../../components/ui/Toast";
import {
  connectionKeys,
  connectionsApi,
  connectionsChanged,
  projectConnectionState,
  useProjectConnections,
  type AvailableConnection,
  type ProjectConnection,
  type ProjectConnections,
} from "../../lib/connections";
import { queryClient, useProjectStatus } from "../../lib/queries";
import type { Project } from "../../lib/types";
import { useProjectContext } from "./ProjectContext";

/* Same page frame as the other settings pages (Settings.tsx). */
function useSettingsPage(title: string): Project | undefined {
  const { slug } = useProjectContext();
  const project = useProjectStatus(slug).data?.project;
  useCrumbs([
    { label: project?.name ?? slug, to: `/projects/${slug}` },
    { label: "Settings", to: `/projects/${slug}/settings` },
    { label: title },
  ]);
  return project;
}

function Header({ title, subtitle }: { title: string; subtitle: string }) {
  return (
    <div className="page-header">
      <div>
        <h1 className="page-title">{title}</h1>
        <p className="page-subtitle">{subtitle}</p>
      </div>
    </div>
  );
}

function EnabledRow({
  item,
  archived,
  pending,
  onDisable,
}: {
  item: ProjectConnection;
  archived: boolean;
  pending: boolean;
  onDisable: () => void;
}) {
  const state = projectConnectionState(item);
  return (
    <div className="row-link resource-row connection-row">
      <div className="connection-main">
        <div className="connection-heading">
          <span className="connection-name">{item.name}</span>
          <Tag tone={state.tone}>{state.label}</Tag>
        </div>
        <div className="mono muted resource-value" title={item.server_url}>
          {item.server_url}
        </div>
        <div className="connection-meta">
          <span>
            MCP server <span className="mono">{item.server_key}</span>
          </span>
          <span>Owner {item.owned_by_you ? "You" : item.owner}</span>
          <span>
            Enabled by {item.enabled_by} <Time iso={item.enabled_at} />
          </span>
        </div>
        {state.detail && (
          <div className="notice" data-tone="warning" role="status">
            <WarningAltFilled size={16} />
            <div>
              {state.detail}
              {item.owned_by_you && item.inactive_reason === "not_connected" && (
                <>
                  {" "}
                  <Link className="link" to="/connections">
                    Open Connections
                  </Link>
                </>
              )}
            </div>
          </div>
        )}
      </div>
      {item.owned_by_you && (
        <div className="connection-actions">
          <button type="button" className="btn btn-sm" onClick={onDisable} disabled={archived || pending}>
            {pending && <span className="spinner" />} Disable
          </button>
        </div>
      )}
    </div>
  );
}

function AvailableRow({
  item,
  archived,
  pending,
  onEnable,
}: {
  item: AvailableConnection;
  archived: boolean;
  pending: boolean;
  onEnable: () => void;
}) {
  const connected = item.status === "CONNECTED";
  const hintId = `connection-hint-${item.connection_id}`;
  return (
    <div className="row-link resource-row connection-row">
      <div className="connection-main">
        <div className="connection-heading">
          <span className="connection-name">{item.name}</span>
          <ConnectionStatusTag status={item.status} />
        </div>
        <div className="mono muted resource-value" title={item.server_url}>
          {item.server_url}
        </div>
        {!connected && (
          <div className="connection-meta" id={hintId}>
            <span>
              Connect it in{" "}
              <Link className="link" to="/connections">
                Connections
              </Link>{" "}
              before enabling it here.
            </span>
          </div>
        )}
      </div>
      <div className="connection-actions">
        <button
          type="button"
          className="btn btn-primary btn-sm"
          onClick={onEnable}
          disabled={!connected || archived || pending}
          aria-describedby={connected ? undefined : hintId}
        >
          {pending && <span className="spinner" />} Enable
        </button>
      </div>
    </div>
  );
}

export function SettingsConnections() {
  const { slug } = useProjectContext();
  const project = useSettingsPage("Connections");
  const data = useProjectConnections(slug);
  const toast = useToast();
  const archived = project?.status === "ARCHIVED";

  const update = (change: (current: ProjectConnections) => ProjectConnections) =>
    queryClient.setQueryData<ProjectConnections>(connectionKeys.project(slug), (current) =>
      current ? change(current) : current,
    );

  const enable = useMutation({
    mutationFn: (item: AvailableConnection) => connectionsApi.enable(slug, item.connection_id),
    onSuccess: (enabled) => {
      update((current) => ({
        ...current,
        connections: [...current.connections.filter((item) => item.connection_id !== enabled.connection_id), enabled],
        available: current.available.filter((item) => item.connection_id !== enabled.connection_id),
      }));
      void connectionsChanged();
      toast(`${enabled.name} enabled`, { tone: "success", body: "This project can use it from the next turn." });
    },
    onError: (error, item) => toast(`Couldn’t enable ${item.name}`, { tone: "critical", body: (error as Error).message }),
  });
  const disable = useMutation({
    mutationFn: (item: ProjectConnection) => connectionsApi.disable(slug, item.connection_id),
    onSuccess: (disabled) => {
      update((current) => ({
        ...current,
        connections: current.connections.filter((item) => item.connection_id !== disabled.connection_id),
        available:
          current.can_enable && disabled.owned_by_you
            ? [
                ...current.available.filter((item) => item.connection_id !== disabled.connection_id),
                {
                  connection_id: disabled.connection_id,
                  name: disabled.name,
                  server_url: disabled.server_url,
                  status: disabled.status,
                },
              ]
            : current.available,
      }));
      void connectionsChanged();
      toast(`${disabled.name} disabled`, { body: "This project will stop using it from the next turn." });
    },
    onError: (error, item) => toast(`Couldn’t disable ${item.name}`, { tone: "critical", body: (error as Error).message }),
  });

  const enabled = data.data?.connections ?? [];
  const available = data.data?.available ?? [];
  const ownsEnabled = enabled.some((item) => item.owned_by_you);

  return (
    <div className="page page-narrow">
      <Header
        title="Connections"
        subtitle="Which of your connections this project may use. Changes apply from the next turn. A running turn keeps its current connections."
      />
      {data.isPending ? (
        <Skeleton height={160} />
      ) : data.error ? (
        <div className="notice" data-tone="critical" role="alert">
          <WarningAltFilled size={16} />
          <div style={{ flex: 1 }}>
            <div className="notice-title">Couldn’t load this project’s connections</div>
            <div className="notice-body">{data.error.message}</div>
          </div>
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => data.refetch()}>
            Try again
          </button>
        </div>
      ) : (
        <>
          {enabled.length ? (
            <div className="panel" aria-label="Enabled in this project">
              <div className="panel-header">
                <span className="panel-title">Enabled in this project</span>
                <span className="muted mono">{enabled.length}</span>
              </div>
              {enabled.map((item) => (
                <EnabledRow
                  key={item.connection_id}
                  item={item}
                  archived={archived}
                  pending={disable.isPending && disable.variables?.connection_id === item.connection_id}
                  onDisable={() => disable.mutate(item)}
                />
              ))}
            </div>
          ) : (
            <Empty icon={<Plug size={20} />} title="No connections enabled">
              This project uses none of its members’ connections.{" "}
              {data.data?.can_enable ? "Enable one of yours below." : "Project owners can enable their own."}
            </Empty>
          )}

          {data.data?.can_enable ? (
            <section className="section">
              <div className="section-header">
                <h2 className="section-title">Your connections</h2>
                <Link to="/connections" className="section-link">
                  Add or connect more <ArrowRight size={14} />
                </Link>
              </div>
              {available.length ? (
                <div className="panel" aria-label="Your connections">
                  {available.map((item) => (
                    <AvailableRow
                      key={item.connection_id}
                      item={item}
                      archived={archived}
                      pending={enable.isPending && enable.variables?.connection_id === item.connection_id}
                      onEnable={() => enable.mutate(item)}
                    />
                  ))}
                </div>
              ) : (
                <Empty
                  title={ownsEnabled ? "All your connections are enabled here" : "You have no connections yet"}
                  action={
                    <Link to="/connections" className="btn btn-sm">
                      Open Connections
                    </Link>
                  }
                >
                  {ownsEnabled
                    ? "Add more MCP servers in Connections."
                    : "Add an MCP server in Connections and connect it, then enable it here."}
                </Empty>
              )}
            </section>
          ) : (
            <p className="muted" style={{ marginTop: "var(--space-5)" }}>
              Only project owners can enable their own connections here.
            </p>
          )}
        </>
      )}
    </div>
  );
}
