import { useMutation } from "@tanstack/react-query";
import { api } from "../../lib/api";
import { keys, queryClient } from "../../lib/queries";
import type { LifecycleAction } from "../../lib/types";
import { useToast } from "../ui/Toast";

const PAST: Record<LifecycleAction, string> = {
  start: "Project started",
  pause: "Project paused",
  resume: "Project resumed",
  stop: "Project stopped",
  archive: "Project archived",
};

export function useLifecycle(slug: string) {
  const toast = useToast();
  return useMutation({
    mutationFn: (action: LifecycleAction) => api.lifecycle(slug, action),
    onSuccess: (project, action) => {
      queryClient.invalidateQueries({ queryKey: ["project", slug] });
      queryClient.invalidateQueries({ queryKey: keys.projects });
      queryClient.setQueryData(keys.status(slug), (current: unknown) =>
        current && typeof current === "object" ? { ...current, project } : current,
      );
      toast(PAST[action], { tone: "success" });
    },
    onError: (error) => toast("Action failed", { body: (error as Error).message, tone: "critical" }),
  });
}
