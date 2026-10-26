import { OverflowMenuHorizontal, Pause, Play, Renew, Stop, Archive } from "@carbon/icons-react";
import * as Dropdown from "@radix-ui/react-dropdown-menu";
import * as Dialog from "@radix-ui/react-dialog";
import { useState } from "react";
import { useNavigate } from "react-router";
import type { LifecycleAction, Project } from "../../lib/types";
import { KickoffDialog } from "./Kickoff";
import { useLifecycle } from "./useLifecycle";
export { useLifecycle } from "./useLifecycle";

interface PrimaryAction {
  action: LifecycleAction;
  label: string;
  icon: React.ReactNode;
  primary: boolean;
}

function primaryAction(project: Project): PrimaryAction | null {
  switch (project.status) {
    case "CREATED":
      return { action: "start", label: "Start", icon: <Play size={16} />, primary: true };
    case "STOPPED":
      return { action: "start", label: "Start", icon: <Play size={16} />, primary: true };
    case "RUNNING":
    case "WAITING":
      return { action: "pause", label: "Pause", icon: <Pause size={16} />, primary: false };
    case "PAUSED":
      return { action: "resume", label: "Resume", icon: <Play size={16} />, primary: true };
    case "FAILED":
      return { action: "resume", label: "Retry", icon: <Renew size={16} />, primary: true };
    default:
      return null;
  }
}

/** The single most relevant lifecycle action, plus stop/archive behind an overflow menu. */
export function LifecycleControls({ project, size }: { project: Project; size?: "sm" }) {
  const lifecycle = useLifecycle(project.slug);
  const navigate = useNavigate();
  const [kickoff, setKickoff] = useState(false);
  const [confirm, setConfirm] = useState<"stop" | "archive" | null>(null);
  if (project.role === "VIEWER") return null;
  const primary = primaryAction(project);
  const canStop = ["CREATED", "RUNNING", "WAITING", "PAUSED", "FAILED"].includes(project.status);
  const canArchive = project.role === "OWNER" && project.status !== "ARCHIVED";
  const sizeClass = size === "sm" ? " btn-sm" : "";

  return (
    <div className="cluster" style={{ flexWrap: "nowrap" }}>
      {primary && (
        <button
          className={`btn${primary.primary ? " btn-primary" : ""}${sizeClass}`}
          disabled={lifecycle.isPending}
          onClick={() => project.status === "CREATED" ? setKickoff(true) : lifecycle.mutate(primary.action)}
        >
          {lifecycle.isPending ? <span className="spinner" /> : primary.icon}
          {primary.label}
        </button>
      )}
      {(canStop || canArchive) && (
        <Dropdown.Root>
          <Dropdown.Trigger className={`btn btn-icon${sizeClass}`} aria-label="More actions">
            <OverflowMenuHorizontal size={16} />
          </Dropdown.Trigger>
          <Dropdown.Portal>
            <Dropdown.Content className="floating" align="end" sideOffset={6}>
              {canStop && (
                <Dropdown.Item className="menu-item" onSelect={() => setConfirm("stop")}>
                  <Stop size={16} /> Stop project
                </Dropdown.Item>
              )}
              {canArchive && (
                <Dropdown.Item className="menu-item" onSelect={() => setConfirm("archive")}>
                  <Archive size={16} /> Archive project
                </Dropdown.Item>
              )}
            </Dropdown.Content>
          </Dropdown.Portal>
        </Dropdown.Root>
      )}
      <KickoffDialog project={project} open={kickoff} onOpenChange={setKickoff} />
      <Dialog.Root open={confirm !== null} onOpenChange={(open) => !open && setConfirm(null)}>
        <Dialog.Portal>
          <Dialog.Overlay className="overlay" />
          <Dialog.Content className="dialog">
            <div className="dialog-header">
              <Dialog.Title className="dialog-title">
                {confirm === "archive" ? "Archive this project?" : "Stop this project?"}
              </Dialog.Title>
              <Dialog.Description className="dialog-description">
                {confirm === "archive"
                  ? "Archiving stops all work and makes the project read-only. Its knowledge and conversation stay available."
                  : "Stopping interrupts the current turn. Knowledge is kept, and you can start the project again later."}
              </Dialog.Description>
            </div>
            <div className="dialog-footer" style={{ marginTop: "var(--space-5)" }}>
              <Dialog.Close className="btn">Cancel</Dialog.Close>
              <button
                className="btn btn-primary"
                disabled={lifecycle.isPending}
                onClick={() => {
                  const action = confirm!;
                  lifecycle.mutate(action, {
                    onSuccess: () => {
                      setConfirm(null);
                      if (action === "archive") navigate("/projects");
                    },
                  });
                }}
              >
                {confirm === "archive" ? "Archive" : "Stop"}
              </button>
            </div>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    </div>
  );
}
