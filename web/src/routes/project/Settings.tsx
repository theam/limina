import { Add, Launch, Locked, TrashCan, Upload } from "@carbon/icons-react";
import * as Dialog from "@radix-ui/react-dialog";
import { useMutation } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";
import { useCrumbs } from "../../components/shell/Crumbs";
import { KickoffDialog } from "../../components/project/Kickoff";
import { ModelFields, projectModelChoice, type ModelChoice } from "../../components/project/ModelFields";
import { EngineAccountPanel } from "../../components/project/EngineAccount";
import { useLifecycle } from "../../components/project/Lifecycle";
import { Empty, Skeleton } from "../../components/ui/Empty";
import { Tag } from "../../components/ui/Status";
import { Time } from "../../components/ui/Time";
import { useToast } from "../../components/ui/Toast";
import { api } from "../../lib/api";
import { engineLabel } from "../../lib/format";
import {
  keys,
  queryClient,
  useEngineAccounts,
  useEngines,
  useProjectStatus,
  useResources,
  useSources,
} from "../../lib/queries";
import type { Engine, Project } from "../../lib/types";
import { useProjectContext } from "./ProjectContext";

function useSettingsPage(title: string): Project | undefined {
  const { slug } = useProjectContext();
  const status = useProjectStatus(slug);
  const project = status.data?.project;
  useCrumbs([
    { label: project?.name ?? slug, to: `/projects/${slug}` },
    { label: "Settings", to: `/projects/${slug}/settings` },
    ...(title === "General" ? [] : [{ label: title }]),
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

function Confirm({
  open,
  onOpenChange,
  title,
  body,
  action,
  onConfirm,
  pending,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  body: string;
  action: string;
  onConfirm: () => void;
  pending?: boolean;
}) {
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="overlay" />
        <Dialog.Content className="dialog">
          <div className="dialog-header">
            <Dialog.Title className="dialog-title">{title}</Dialog.Title>
            <Dialog.Description className="dialog-description">{body}</Dialog.Description>
          </div>
          <div className="dialog-footer" style={{ marginTop: "var(--space-5)" }}>
            <Dialog.Close className="btn">Cancel</Dialog.Close>
            <button className="btn btn-primary" onClick={onConfirm} disabled={pending}>
              {pending && <span className="spinner" />}
              {action}
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

/* ─── General ───────────────────────────────────────────────────────── */

export function SettingsGeneral() {
  const { slug } = useProjectContext();
  const project = useSettingsPage("General");
  const toast = useToast();
  const navigate = useNavigate();
  const lifecycle = useLifecycle(slug);
  const [form, setForm] = useState({ name: "", mission: "", success_criteria: "", context: "" });
  const [confirm, setConfirm] = useState<"stop" | "archive" | null>(null);
  const loadedFor = useRef<string | null>(null);

  // Load the saved values once per project; live refetches must not overwrite an edit in progress.
  const reset = (source: Project) =>
    setForm({
      name: source.name,
      mission: source.mission,
      success_criteria: source.success_criteria,
      context: source.context,
    });
  useEffect(() => {
    if (project && loadedFor.current !== project.slug) {
      loadedFor.current = project.slug;
      reset(project);
    }
  }, [project]);

  const save = useMutation({
    mutationFn: () => api.updateProject(slug, form),
    onSuccess: (saved) => {
      reset(saved);
      queryClient.invalidateQueries({ queryKey: ["project", slug] });
      queryClient.invalidateQueries({ queryKey: keys.projects });
      toast("Project updated", { tone: "success" });
    },
    onError: (error) => toast("Couldn’t save", { body: (error as Error).message, tone: "critical" }),
  });

  if (!project) return <div className="page page-narrow"><Skeleton height={320} /></div>;
  const editable = project.role !== "VIEWER" && project.status !== "ARCHIVED";
  const dirty =
    form.name !== project.name ||
    form.mission !== project.mission ||
    form.success_criteria !== project.success_criteria ||
    form.context !== project.context;
  const set = (name: keyof typeof form) => (event: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) =>
    setForm((current) => ({ ...current, [name]: event.target.value }));

  return (
    <div className="page page-narrow">
      <Header title="Settings" subtitle="The mission Limina works toward. Changes apply from the next turn." />
      <form
        className="panel"
        onSubmit={(event) => {
          event.preventDefault();
          save.mutate();
        }}
      >
        <div className="panel-body form-grid">
          <label className="field">
            <span className="field-label">Name</span>
            <input className="input" value={form.name} onChange={set("name")} disabled={!editable} required />
          </label>
          <label className="field">
            <span className="field-label">Mission</span>
            <textarea className="textarea" rows={4} value={form.mission} onChange={set("mission")} disabled={!editable} required />
          </label>
          <label className="field">
            <span className="field-label">Success criteria</span>
            <textarea className="textarea" rows={3} value={form.success_criteria} onChange={set("success_criteria")} disabled={!editable} required />
          </label>
          <label className="field">
            <span className="field-label">Context</span>
            <textarea className="textarea" rows={3} value={form.context} onChange={set("context")} disabled={!editable} />
          </label>
        </div>
        <div className="panel-footer">
          <span className="mono">{slug}</span>
          <button className="btn btn-primary btn-sm" type="submit" disabled={!editable || !dirty || save.isPending}>
            {save.isPending && <span className="spinner" />} Save changes
          </button>
        </div>
      </form>

      {editable && (
        <section className="section">
          <div className="section-header">
            <h2 className="section-title">Danger zone</h2>
          </div>
          <div className="panel danger">
            <div className="danger-row">
              <div>
                <div style={{ fontWeight: 500 }}>Stop project</div>
                <div className="muted">Interrupt the current turn and stop scheduling new ones. You can start again later.</div>
              </div>
              <button className="btn btn-danger btn-sm" onClick={() => setConfirm("stop")} disabled={project.status === "STOPPED" || project.status === "COMPLETE"}>
                Stop
              </button>
            </div>
            {project.role === "OWNER" && (
              <div className="danger-row">
                <div>
                  <div style={{ fontWeight: 500 }}>Archive project</div>
                  <div className="muted">Stop all work and make the project read-only. Knowledge and history are kept.</div>
                </div>
                <button className="btn btn-danger btn-sm" onClick={() => setConfirm("archive")}>
                  Archive
                </button>
              </div>
            )}
          </div>
        </section>
      )}
      <Confirm
        open={confirm !== null}
        onOpenChange={(open) => !open && setConfirm(null)}
        title={confirm === "archive" ? "Archive this project?" : "Stop this project?"}
        body={
          confirm === "archive"
            ? "Archiving stops all work and makes the project read-only."
            : "Stopping interrupts the current turn. Knowledge is kept."
        }
        action={confirm === "archive" ? "Archive" : "Stop"}
        pending={lifecycle.isPending}
        onConfirm={() =>
          lifecycle.mutate(confirm!, {
            onSuccess: () => {
              if (confirm === "archive") navigate("/projects");
              setConfirm(null);
            },
          })
        }
      />
    </div>
  );
}

/* ─── Engine ────────────────────────────────────────────────────────── */

const ENGINES: Engine[] = ["codex", "claude-code"];

export function SettingsEngine() {
  const { slug } = useProjectContext();
  const project = useSettingsPage("Engine");
  const accounts = useEngineAccounts(slug);
  const engines = useEngines();
  const toast = useToast();
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const [choice, setChoice] = useState<(ModelChoice & { runtime: Engine }) | null>(null);
  const [kickoff, setKickoff] = useState(false);
  // `?connect=<engine>&then=start` comes from New Project: connect, then start the first turn.
  const connectEngine = params.get("connect");
  const thenStart = params.get("then") === "start";

  const save = useMutation({
    mutationFn: (settings: ModelChoice & { runtime: Engine }) => api.updateProject(slug, {
      ...(settings.runtime !== project?.runtime ? { runtime: settings.runtime } : {}),
      model: settings.model, reasoning_effort: settings.reasoning_effort,
    }),
    onSuccess: (saved) => {
      queryClient.setQueryData(keys.status(slug), (current: unknown) =>
        current && typeof current === "object" ? { ...current, project: saved } : current);
      setChoice(null);
      queryClient.invalidateQueries({ queryKey: ["project", slug] });
      queryClient.invalidateQueries({ queryKey: keys.projects });
      toast("Project defaults saved", { tone: "success", body: "Applies from the next turn." });
    },
    onError: (error) => toast("Couldn’t save defaults", { body: (error as Error).message, tone: "critical" }),
  });


  if (!project) return <div className="page page-narrow"><Skeleton height={320} /></div>;
  const owner = project.role === "OWNER" && project.status !== "ARCHIVED";
  const editable = owner && !save.isPending;
  const canSwitch = editable && project.status === "CREATED";
  const settings = choice ?? { runtime: project.runtime, ...projectModelChoice(project) };
  const selected = settings.runtime;
  const dirty = selected !== project.runtime || settings.model !== (project.model ?? "") || settings.reasoning_effort !== (project.reasoning_effort ?? "");
  const ordered = [...(accounts.data ?? [])].sort(
    (a, b) => Number(b.engine === project.runtime) - Number(a.engine === project.runtime),
  );
  const onConnected = () => {
    if (thenStart) setKickoff(true);
    else if (connectEngine) setParams({}, { replace: true });
  };

  return (
    <div className="page page-narrow">
      <Header
        title="Engine"
        subtitle="The engine, default model, reasoning mode and account for this project."
      />
      {thenStart && (
        <div className="notice" data-tone="info" style={{ marginBottom: "var(--space-5)" }}>
          <div style={{ flex: 1 }}>
            <div className="notice-title">Connect an account to start {project.name}</div>
            <div className="notice-body">After connecting, review the model and reasoning mode to start the first turn.</div>
          </div>
          <button className="btn btn-ghost btn-sm" onClick={() => setParams({}, { replace: true })}>
            Later
          </button>
        </div>
      )}
      <form
        className="panel"
        onSubmit={(event) => {
          event.preventDefault();
          if (dirty) save.mutate(settings);
        }}
      >
        <div className="panel-body">
          <div className="engine-grid" role="radiogroup" aria-label="Engine">
            {ENGINES.map((engine) => (
              <label key={engine} className="engine-option" data-selected={selected === engine} data-disabled={!canSwitch}>
                <input
                  type="radio"
                  name="engine"
                  value={engine}
                  checked={selected === engine}
                  disabled={!canSwitch}
                  onChange={() => setChoice({ runtime: engine, model: "", reasoning_effort: "" })}
                  className="sr-only"
                />
                <span className="engine-option-name">{engineLabel(engine)}</span>
                <span className="muted">
                  {engines.data?.find((item) => item.engine === engine)?.installed === false
                    ? "Not installed on this runtime."
                    : engine === project.runtime
                      ? "Runs this project."
                      : "Available."}
                </span>
              </label>
            ))}
          </div>
          <div style={{ marginTop: "var(--space-5)" }}>
            <ModelFields key={selected} slug={slug} engine={selected} value={settings} disabled={!editable}
              onChange={(value) => setChoice({ runtime: selected, ...value })} />
          </div>
        </div>
        <div className="panel-footer">
          <span>{project.status === "CREATED" ? "Choose the engine before kickoff." : "Model and reasoning changes apply from the next turn."}</span>
          {dirty && (
            <button className="btn btn-primary btn-sm" type="submit" disabled={!editable || save.isPending}>
              {save.isPending && <span className="spinner" />} Save defaults
            </button>
          )}
        </div>
      </form>

      <KickoffDialog project={project} open={kickoff} onOpenChange={(open) => {
        setKickoff(open);
        if (!open) setParams({}, { replace: true });
      }} onStarted={() => navigate(`/projects/${slug}/conversation`)} />

      <section className="section">
        <div className="section-header">
          <h2 className="section-title">Accounts</h2>
        </div>
        <p className="muted" style={{ marginBottom: "var(--space-4)" }}>
          Connect your own subscription so this project runs on it. Each project keeps its own account.
        </p>
        {accounts.isPending ? (
          <Skeleton height={160} />
        ) : (
          <div className="stack" style={{ gap: "var(--space-5)" }}>
            {ordered.map((item) => (
              <EngineAccountPanel
                key={item.engine}
                slug={slug}
                item={item}
                owner={owner}
                inUse={item.engine === project.runtime}
                autoStart={connectEngine === item.engine}
                onConnected={item.engine === project.runtime ? onConnected : undefined}
              />
            ))}
          </div>
        )}
      </section>
    </div>
  );
}

/* ─── Resources ─────────────────────────────────────────────────────── */

export function SettingsResources() {
  const { slug } = useProjectContext();
  const project = useSettingsPage("Resources");
  const resources = useResources(slug);
  const toast = useToast();
  const [type, setType] = useState<"VARIABLE" | "SECRET">("VARIABLE");
  const [name, setName] = useState("");
  const [value, setValue] = useState("");
  const [removing, setRemoving] = useState<string | null>(null);
  const editable = project?.role !== "VIEWER";

  const add = useMutation({
    mutationFn: () => (type === "SECRET" ? api.setSecret(slug, name, value) : api.setVariable(slug, name, value)),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: keys.resources(slug) });
      toast(type === "SECRET" ? "Secret saved" : "Variable saved", { tone: "success", body: name });
      setName("");
      setValue("");
    },
    onError: (error) => toast("Couldn’t save", { body: (error as Error).message, tone: "critical" }),
  });
  const remove = useMutation({
    mutationFn: (resource: string) => api.removeResource(slug, resource),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: keys.resources(slug) });
      setRemoving(null);
    },
  });

  const items = (resources.data ?? []).filter((item) => item.status !== "REMOVED");

  return (
    <div className="page page-narrow">
      <Header
        title="Resources"
        subtitle="Environment variables and write-only secrets available to the engine. Changes restart the active turn."
      />
      {editable && (
        <form
          className="panel"
          style={{ marginBottom: "var(--space-5)" }}
          onSubmit={(event) => {
            event.preventDefault();
            add.mutate();
          }}
        >
          <div className="panel-body form-grid" style={{ gap: "var(--space-4)" }}>
            <div className="segmented" role="group" aria-label="Resource type" style={{ justifySelf: "start" }}>
              <button type="button" aria-pressed={type === "VARIABLE"} onClick={() => setType("VARIABLE")}>
                Variable
              </button>
              <button type="button" aria-pressed={type === "SECRET"} onClick={() => setType("SECRET")}>
                Secret
              </button>
            </div>
            <div className="form-row">
              <label className="field">
                <span className="field-label">Name</span>
                <input
                  className="input mono"
                  value={name}
                  required
                  pattern="[A-Za-z_][A-Za-z0-9_]*"
                  placeholder="DATASET_URL"
                  onChange={(event) => setName(event.target.value.toUpperCase().replace(/[^A-Z0-9_]/g, "_"))}
                />
              </label>
              <label className="field">
                <span className="field-label">Value</span>
                <input
                  className="input"
                  type={type === "SECRET" ? "password" : "text"}
                  autoComplete="off"
                  value={value}
                  required
                  onChange={(event) => setValue(event.target.value)}
                  placeholder={type === "SECRET" ? "Stored encrypted, never shown again" : "https://…"}
                />
              </label>
            </div>
          </div>
          <div className="panel-footer">
            <span>
              {type === "SECRET"
                ? "Secrets are encrypted at rest and never returned by the API."
                : "Variables are visible to project members and to the engine."}
            </span>
            <button className="btn btn-primary btn-sm" disabled={!name || !value || add.isPending}>
              <Add size={16} /> Add
            </button>
          </div>
        </form>
      )}
      {resources.isPending ? (
        <Skeleton height={120} />
      ) : items.length ? (
        <div className="panel">
          {items.map((item) => (
            <div key={item.name} className="row-link resource-row">
              {item.type === "SECRET" ? <Locked size={16} className="muted" /> : <span className="mono muted">$</span>}
              <div style={{ minWidth: 0, flex: 1 }}>
                <div className="mono" style={{ fontWeight: 500 }}>{item.name}</div>
                <div className="muted resource-value">
                  {item.type === "SECRET" ? (item.configured ? "•••••••• configured" : "not configured") : item.value}
                </div>
              </div>
              <Tag>{item.type.toLowerCase()}</Tag>
              <Time iso={item.updated_at} className="muted mono" />
              {editable &&
                (removing === item.name ? (
                  <span className="cluster" style={{ flexWrap: "nowrap" }}>
                    <button className="btn btn-danger btn-sm" onClick={() => remove.mutate(item.name)} disabled={remove.isPending}>
                      Remove
                    </button>
                    <button className="btn btn-ghost btn-sm" onClick={() => setRemoving(null)}>
                      Cancel
                    </button>
                  </span>
                ) : (
                  <button className="btn btn-ghost btn-icon btn-sm" aria-label={`Remove ${item.name}`} onClick={() => setRemoving(item.name)}>
                    <TrashCan size={16} />
                  </button>
                ))}
            </div>
          ))}
        </div>
      ) : (
        <Empty title="No resources">Add API endpoints, dataset locations, or credentials the engine needs.</Empty>
      )}
    </div>
  );
}

/* ─── Sources ───────────────────────────────────────────────────────── */

export function SettingsSources() {
  const { slug } = useProjectContext();
  const project = useSettingsPage("Sources");
  const sources = useSources(slug);
  const toast = useToast();
  const [mode, setMode] = useState<"URL" | "UPLOAD">("URL");
  const [name, setName] = useState("");
  const [uri, setUri] = useState("");
  const [file, setFile] = useState<File | null>(null);
  const editable = project?.role !== "VIEWER";

  const add = useMutation({
    mutationFn: () => (mode === "UPLOAD" && file ? api.uploadSource(slug, name, file) : api.setSource(slug, { name, type: "URL", uri })),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: keys.sources(slug) });
      toast("Source added", { tone: "success", body: name });
      setName("");
      setUri("");
      setFile(null);
    },
    onError: (error) => toast("Couldn’t add source", { body: (error as Error).message, tone: "critical" }),
  });
  const remove = useMutation({
    mutationFn: (id: string) => api.removeSource(slug, id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.sources(slug) }),
  });
  const items = (sources.data ?? []).filter((item) => item.status === "ACTIVE");

  return (
    <div className="page page-narrow">
      <Header title="Sources" subtitle="Documents, datasets and links Limina should read. Each turn sees the current list." />
      {editable && (
        <form
          className="panel"
          style={{ marginBottom: "var(--space-5)" }}
          onSubmit={(event) => {
            event.preventDefault();
            add.mutate();
          }}
        >
          <div className="panel-body form-grid" style={{ gap: "var(--space-4)" }}>
            <div className="segmented" role="group" aria-label="Source type" style={{ justifySelf: "start" }}>
              <button type="button" aria-pressed={mode === "URL"} onClick={() => setMode("URL")}>
                Link
              </button>
              <button type="button" aria-pressed={mode === "UPLOAD"} onClick={() => setMode("UPLOAD")}>
                Upload
              </button>
            </div>
            <div className="form-row">
              <label className="field">
                <span className="field-label">Name</span>
                <input className="input" value={name} required onChange={(event) => setName(event.target.value)} placeholder="Evaluation dataset" />
              </label>
              {mode === "URL" ? (
                <label className="field">
                  <span className="field-label">URL</span>
                  <input className="input" type="url" value={uri} required onChange={(event) => setUri(event.target.value)} placeholder="https://…" />
                </label>
              ) : (
                <label className="field">
                  <span className="field-label">File</span>
                  <input
                    className="input"
                    type="file"
                    required
                    onChange={(event) => {
                      const selected = event.target.files?.[0] ?? null;
                      setFile(selected);
                      if (selected && !name) setName(selected.name);
                    }}
                  />
                </label>
              )}
            </div>
          </div>
          <div className="panel-footer">
            <span>{mode === "URL" ? "Links must not embed credentials; use a secret instead." : "Uploads are stored in the project workspace."}</span>
            <button className="btn btn-primary btn-sm" disabled={!name || (mode === "URL" ? !uri : !file) || add.isPending}>
              {mode === "UPLOAD" ? <Upload size={16} /> : <Add size={16} />} Add source
            </button>
          </div>
        </form>
      )}
      {sources.isPending ? (
        <Skeleton height={120} />
      ) : items.length ? (
        <div className="panel">
          {items.map((item) => (
            <div key={item.id} className="row-link resource-row">
              <div style={{ minWidth: 0, flex: 1 }}>
                <div style={{ fontWeight: 500 }}>{item.name}</div>
                <div className="muted resource-value">
                  {item.type === "URL" ? (
                    <a href={item.uri} target="_blank" rel="noreferrer" className="link">
                      {item.uri} <Launch size={12} style={{ display: "inline" }} />
                    </a>
                  ) : (
                    String(item.metadata.filename ?? item.uri)
                  )}
                </div>
              </div>
              <Tag>{item.type.toLowerCase()}</Tag>
              <Time iso={item.updated_at} className="muted mono" />
              {editable && (
                <button className="btn btn-ghost btn-icon btn-sm" aria-label={`Remove ${item.name}`} onClick={() => remove.mutate(item.id)}>
                  <TrashCan size={16} />
                </button>
              )}
            </div>
          ))}
        </div>
      ) : (
        <Empty title="No sources yet">Add papers, datasets, dashboards or repositories that ground the research.</Empty>
      )}
    </div>
  );
}

export { SettingsMembers } from "../../components/project/Sharing";
