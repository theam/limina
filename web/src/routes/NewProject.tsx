import { WarningAltFilled } from "@carbon/icons-react";
import { useMutation } from "@tanstack/react-query";
import { useState } from "react";
import { Link, useNavigate } from "react-router";
import { useCrumbs } from "../components/shell/Crumbs";
import { ModelFields } from "../components/project/ModelFields";
import { useToast } from "../components/ui/Toast";
import { api } from "../lib/api";
import { engineLabel, slugify } from "../lib/format";
import { keys, queryClient, useEngines, useTemplates } from "../lib/queries";
import type { Engine } from "../lib/types";

const ENGINES: { id: Engine; blurb: string }[] = [
  { id: "codex", blurb: "OpenAI Codex with Limina-owned threads and steering." },
  { id: "claude-code", blurb: "Claude with subagents in a sandboxed workspace." },
];

export function NewProject() {
  useCrumbs([{ label: "Projects", to: "/projects" }, { label: "New project" }]);
  const navigate = useNavigate();
  const toast = useToast();
  const engines = useEngines();
  const templates = useTemplates();
  const [name, setName] = useState("");
  const [slug, setSlug] = useState("");
  const [slugTouched, setSlugTouched] = useState(false);
  const [objective, setObjective] = useState("");
  const [criteria, setCriteria] = useState("");
  const [context, setContext] = useState("");
  const [engine, setEngine] = useState<Engine>("codex");
  const [model, setModel] = useState({ model: "", reasoning_effort: "" });
  const [startNow, setStartNow] = useState(true);
  const [template, setTemplate] = useState<string | null>(null);
  const [preferOwn, setPreferOwn] = useState(true);

  const effectiveSlug = slugTouched ? slug : slugify(name);
  const engineInfo = (id: Engine) => engines.data?.find((item) => item.engine === id);
  const shared = Boolean(engineInfo(engine)?.installed && engineInfo(engine)?.credentials);
  // Without shared runtime credentials, the project can only run on its own account.
  const ownAccount = preferOwn || !shared;

  const create = useMutation({
    mutationFn: async () => {
      const project = await api.createProject({
        slug: effectiveSlug,
        name: name.trim(),
        objective: objective.trim(),
        success_criteria: criteria.trim(),
        context: context.trim(),
        runtime: engine,
        ...model,
      });
      // Your own account: sign in right after creating, then the first turn starts.
      if (ownAccount) return { project, connect: true };
      if (startNow) await api.lifecycle(project.slug, "start");
      return { project, connect: false };
    },
    onSuccess: ({ project, connect }) => {
      queryClient.invalidateQueries({ queryKey: keys.projects });
      if (connect) {
        const query = new URLSearchParams({ connect: engine, ...(startNow ? { then: "start" } : {}) });
        navigate(`/projects/${project.slug}/settings/engine?${query}`);
        return;
      }
      toast(startNow ? "Project started" : "Project created", { tone: "success", body: project.name });
      navigate(startNow ? `/projects/${project.slug}/conversation` : `/projects/${project.slug}`);
    },
  });

  const valid = name.trim() && effectiveSlug && objective.trim() && criteria.trim();
  const submitLabel = ownAccount ? "Create and sign in" : startNow ? "Create and start" : "Create project";

  return (
    <div className="page page-narrow">
      <div className="page-header">
        <div>
          <h1 className="page-title">New project</h1>
          <p className="page-subtitle">
            Frame the mission once. Limina keeps it running, records the evidence, and asks you when a
            decision is needed.
          </p>
        </div>
      </div>

      <form
        className="form-grid"
        onSubmit={(event) => {
          event.preventDefault();
          if (valid) create.mutate();
        }}
      >
        {templates.data && templates.data.length > 0 && (
          <div className="field">
            <span className="field-label">Start from</span>
            <div className="cluster">
              {templates.data.map((item) => (
                <button
                  type="button"
                  key={item.id}
                  className="choice-chip"
                  aria-pressed={template === item.id}
                  title={item.description}
                  onClick={() => {
                    setTemplate(item.id);
                    setCriteria(item.defaults.success_criteria);
                    setContext(item.defaults.context);
                  }}
                >
                  {item.name}
                </button>
              ))}
            </div>
          </div>
        )}

        <div className="form-row">
          <label className="field">
            <span className="field-label">Name</span>
            <input
              className="input"
              value={name}
              autoFocus
              required
              maxLength={240}
              onChange={(event) => setName(event.target.value)}
              placeholder="Retrieval reliability lab"
            />
          </label>
          <label className="field">
            <span className="field-label">Slug</span>
            <input
              className="input mono"
              value={effectiveSlug}
              required
              pattern="[a-z0-9]+(-[a-z0-9]+)*"
              maxLength={120}
              onChange={(event) => {
                setSlugTouched(true);
                setSlug(slugify(event.target.value) || event.target.value.toLowerCase());
              }}
              placeholder="retrieval-reliability"
            />
          </label>
        </div>

        <label className="field">
          <span className="field-label">Mission</span>
          <textarea
            className="textarea"
            value={objective}
            required
            rows={4}
            onChange={(event) => setObjective(event.target.value)}
            placeholder="What should Limina figure out or build? Be specific about the problem, not the method."
          />
        </label>

        <label className="field">
          <span className="field-label">Success criteria</span>
          <textarea
            className="textarea"
            value={criteria}
            required
            rows={3}
            onChange={(event) => setCriteria(event.target.value)}
            placeholder="How will you know it worked? Name the metric, threshold, or decision."
          />
        </label>

        <label className="field">
          <span className="field-label">
            Context <span className="muted">— optional</span>
          </span>
          <textarea
            className="textarea"
            value={context}
            rows={3}
            onChange={(event) => setContext(event.target.value)}
            placeholder="Constraints, prior work, data locations, people to ask…"
          />
          <span className="field-hint">You can add sources, variables and secrets after creating the project.</span>
        </label>

        <fieldset className="field" style={{ border: 0, padding: 0, margin: 0 }}>
          <legend className="field-label" style={{ marginBottom: 6 }}>
            Engine
          </legend>
          <div className="engine-grid">
            {ENGINES.map((item) => {
              const info = engineInfo(item.id);
              return (
                <label key={item.id} className="engine-option" data-selected={engine === item.id}>
                  <input
                    type="radio"
                    name="engine"
                    value={item.id}
                    checked={engine === item.id}
                    onChange={() => { setEngine(item.id); setModel({ model: "", reasoning_effort: "" }); }}
                    className="sr-only"
                  />
                  <span className="engine-option-name">{engineLabel(item.id)}</span>
                  <span className="muted">{item.blurb}</span>
                  {info && !info.installed && (
                    <span className="engine-option-state">
                      <WarningAltFilled size={14} /> Not installed on this runtime
                    </span>
                  )}
                </label>
              );
            })}
          </div>
        </fieldset>

        <ModelFields key={engine} engine={engine} value={model} onChange={setModel} disabled={create.isPending} />

        <fieldset className="field" style={{ border: 0, padding: 0, margin: 0 }}>
          <legend className="field-label" style={{ marginBottom: 6 }}>
            Account
          </legend>
          {shared && (
            <div className="segmented" role="group" aria-label="Account" style={{ justifySelf: "start", marginBottom: 8 }}>
              <button type="button" aria-pressed={ownAccount} onClick={() => setPreferOwn(true)}>
                My account
              </button>
              <button type="button" aria-pressed={!ownAccount} onClick={() => setPreferOwn(false)}>
                Shared runtime
              </button>
            </div>
          )}
          {!ownAccount ? (
            <span className="field-hint">Runs on the credentials this runtime shares with every project.</span>
          ) : (
            <span className="field-hint">
              Right after creating the project you’ll sign in with your {engine === "codex" ? "ChatGPT" : "Claude"}{" "}
              account. It takes about a minute, and the sign-in belongs to this project alone.
            </span>
          )}
        </fieldset>

        <label className="checkbox">
          <input type="checkbox" checked={startNow} onChange={(event) => setStartNow(event.target.checked)} />
          <span>
            Start right away
            <span className="field-hint" style={{ display: "block" }}>
              Otherwise the project stays as a draft until you start it.
            </span>
          </span>
        </label>

        {create.error && (
          <div className="notice" data-tone="critical" role="alert">
            {(create.error as Error).message}
          </div>
        )}

        <div className="form-actions" style={{ borderTop: "1px solid var(--hairline)", paddingTop: "var(--space-5)" }}>
          <Link to="/projects" className="btn">
            Cancel
          </Link>
          <button className="btn btn-primary" type="submit" disabled={!valid || create.isPending}>
            {create.isPending && <span className="spinner" />}
            {submitLabel}
          </button>
        </div>
      </form>
    </div>
  );
}
