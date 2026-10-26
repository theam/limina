import { Renew } from "@carbon/icons-react";
import { useId, useState } from "react";
import { useModelCatalog } from "../../lib/queries";
import type { Engine, Project } from "../../lib/types";

export interface ModelChoice {
  model: string;
  reasoning_effort: string;
}

export function projectModelChoice(project: Project): ModelChoice {
  return { model: project.model ?? "", reasoning_effort: project.reasoning_effort ?? "" };
}

/** Provider metadata is the only source of model and reasoning options. */
export function ModelFields({ engine, slug, value, onChange, disabled = false }: {
  engine: Engine;
  slug?: string;
  value: ModelChoice;
  onChange: (value: ModelChoice) => void;
  disabled?: boolean;
}) {
  const id = useId();
  const catalog = useModelCatalog(engine, slug);
  const [customChosen, setCustomChosen] = useState(false);
  const models = catalog.data?.models ?? [];
  const target = value.model || catalog.data?.default_model;
  const selected = target ? models.find((model) => model.id === target || model.resolved_model === target) : undefined;
  const custom = customChosen || Boolean(value.model && catalog.data && !selected);
  const efforts = selected?.reasoning_efforts ?? [];
  const unlistedEffort = Boolean(value.reasoning_effort && !efforts.includes(value.reasoning_effort));
  const unsupported = Boolean(catalog.data && !catalog.isError && unlistedEffort);
  const hintId = `${id}-hint`;

  return (
    <div className="form-grid">
      <div className="form-row">
        <label className="field">
          <span className="field-label">Default model</span>
          <select className="select" disabled={disabled} value={custom ? "__custom" : value.model}
            onChange={(event) => {
              const next = event.target.value;
              setCustomChosen(next === "__custom");
              onChange({ model: next === "__custom" ? "" : next, reasoning_effort: "" });
            }}>
            <option value="">Runtime default</option>
            {value.model && !custom && selected?.id !== value.model && (
              <option value={value.model}>{selected ? `${selected.label} (${value.model})` : `${value.model} (saved)`}</option>
            )}
            {models.map((model) => <option key={model.id} value={model.id}>
              {model.label}{model.resolved_model ? ` · ${model.resolved_model}` : ""}
            </option>)}
            <option value="__custom">Custom model…</option>
          </select>
        </label>
        <label className="field">
          <span className="field-label">Reasoning mode</span>
          <select className="select" disabled={disabled || (!efforts.length && !value.reasoning_effort)}
            value={value.reasoning_effort} aria-describedby={hintId} aria-invalid={unsupported || undefined}
            onChange={(event) => onChange({ ...value, reasoning_effort: event.target.value })}>
            <option value="">Model default{selected?.default_reasoning_effort ? ` (${selected.default_reasoning_effort})` : ""}</option>
            {unlistedEffort && <option value={value.reasoning_effort} disabled>{value.reasoning_effort} ({unsupported ? "not advertised" : "saved"})</option>}
            {efforts.map((effort) => <option key={effort} value={effort}>
              {effort === "xhigh" ? "Extra high" : effort[0].toUpperCase() + effort.slice(1)}
            </option>)}
          </select>
        </label>
      </div>
      {custom && (
        <label className="field">
          <span className="field-label">Model ID</span>
          <input className="input mono" value={value.model} disabled={disabled} required maxLength={200}
            pattern={"\\S+"} placeholder={engine === "codex" ? "Enter a Codex model ID" : "Enter a Claude model ID or alias"}
            onChange={(event) => onChange({ model: event.target.value, reasoning_effort: "" })} />
        </label>
      )}
      <div className="cluster" style={{ justifyContent: "space-between", alignItems: "start" }}>
        <p className="field-hint" id={hintId} style={{ flex: "1 1 220px" }}>
          {catalog.isPending ? "Loading current models from the provider…" : catalog.isError
            ? "Couldn’t refresh provider models. Check the engine connection and try again."
            : selected?.resolved_model ? `Runs ${selected.resolved_model}.` : "Options are loaded from the provider."}
          {!catalog.isPending && !catalog.isError && !efforts.length && " No reasoning modes are advertised for this model."}
          {unsupported && " The saved reasoning mode is no longer advertised. Choose a current mode or the model default."}
        </p>
        <button className="btn btn-ghost btn-sm" type="button" disabled={catalog.isFetching}
          onClick={() => catalog.refetch()} aria-label="Refresh models">
          {catalog.isFetching ? <span className="spinner" /> : <Renew size={14} />} Refresh models
        </button>
      </div>
    </div>
  );
}
