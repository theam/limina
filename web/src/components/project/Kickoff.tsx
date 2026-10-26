import * as Dialog from "@radix-ui/react-dialog";
import { useMutation } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "../../lib/api";
import { engineLabel } from "../../lib/format";
import type { Project } from "../../lib/types";
import { useLifecycle } from "./useLifecycle";
import { ModelFields, projectModelChoice } from "./ModelFields";

function KickoffForm({ project, onStarted }: { project: Project; onStarted: () => void }) {
  const [choice, setChoice] = useState(() => projectModelChoice(project));
  const lifecycle = useLifecycle(project.slug);
  const start = useMutation({
    mutationFn: async () => {
      if (choice.model !== (project.model ?? "") || choice.reasoning_effort !== (project.reasoning_effort ?? "")) {
        await api.updateProject(project.slug, choice);
      }
      await lifecycle.mutateAsync("start");
    },
    onSuccess: onStarted,
  });
  return (
    <form onSubmit={(event) => { event.preventDefault(); start.mutate(); }}>
      <div className="dialog-header">
        <Dialog.Title className="dialog-title">Kick off {project.name}</Dialog.Title>
        <Dialog.Description className="dialog-description">
          Review the defaults for {engineLabel(project.runtime)} before the first turn. You can change them later in Settings → Engine.
        </Dialog.Description>
      </div>
      <div className="dialog-body">
      <p className="muted" style={{ marginBottom: "var(--space-5)" }}>{project.mission}</p>
      <ModelFields slug={project.slug} engine={project.runtime} value={choice} onChange={setChoice} disabled={project.role !== "OWNER" || start.isPending} />
      {start.error && <div className="notice" data-tone="critical" role="alert" style={{ marginTop: 16 }}>{(start.error as Error).message}</div>}
      </div>
      <div className="dialog-footer">
        <Dialog.Close className="btn" disabled={start.isPending}>Cancel</Dialog.Close>
        <button className="btn btn-primary" disabled={start.isPending} type="submit">
          {start.isPending && <span className="spinner" />} Start project
        </button>
      </div>
    </form>
  );
}

export function KickoffDialog({ project, open, onOpenChange, onStarted }: {
  project: Project; open: boolean; onOpenChange: (open: boolean) => void; onStarted?: () => void;
}) {
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="overlay" />
        <Dialog.Content className="dialog" onInteractOutside={(event) => event.preventDefault()}>
          <KickoffForm project={project} onStarted={() => { onOpenChange(false); onStarted?.(); }} />
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
