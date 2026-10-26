import { useQuery } from "@tanstack/react-query";
import { api, request } from "./api";
import { queryClient, keys } from "./queries";
import type { Role } from "./types";

export interface Share {
  subject: string | null;
  display_name: string;
  email: string | null;
  role: Role;
  pending: boolean;
}
export interface SharedProject {
  slug: string;
  owners: { display_name: string }[];
}
const path = (slug: string) => `/v1/projects/${encodeURIComponent(slug)}/sharing`;
export const sharingApi = {
  list: (slug: string) => request<Share[]>(path(slug)),
  share: (slug: string, email: string, role: Role) =>
    request<Share>(path(slug), { method: "PUT", body: { email, role } }),
  remove: (slug: string, share: Share) => share.subject
    ? api.removeMember(slug, share.subject)
    : request<Share>(`${path(slug)}?email=${encodeURIComponent(share.email ?? "")}`, { method: "DELETE" }),
  shared: () => request<SharedProject[]>("/v1/me/shared-projects"),
};
export const sharingKey = (slug: string) => ["project", slug, "sharing"] as const;
export const useSharing = (slug: string) => useQuery({ queryKey: sharingKey(slug), queryFn: () => sharingApi.list(slug) });
export const useSharedProjects = () => useQuery({ queryKey: ["shared-projects"], queryFn: sharingApi.shared });
export async function sharingChanged(slug: string) {
  await Promise.all([
    queryClient.invalidateQueries({ queryKey: sharingKey(slug) }),
    queryClient.invalidateQueries({ queryKey: keys.members(slug) }),
    queryClient.invalidateQueries({ queryKey: keys.projects }),
    queryClient.invalidateQueries({ queryKey: keys.status(slug) }),
    queryClient.invalidateQueries({ queryKey: ["shared-projects"] }),
  ]);
}
