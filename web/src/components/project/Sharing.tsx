import { Add, TrashCan } from "@carbon/icons-react";
import { useMutation } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "../../lib/api";
import { useProjectStatus } from "../../lib/queries";
import { sharingApi, sharingChanged, useSharing, type Share } from "../../lib/sharing";
import type { Role } from "../../lib/types";
import { useProjectContext } from "../../routes/project/ProjectContext";
import { useCrumbs } from "../shell/Crumbs";
import { Skeleton } from "../ui/Empty";
import { Tag } from "../ui/Status";
import { useToast } from "../ui/Toast";

const roles: Role[] = ["OWNER", "EDITOR", "VIEWER"];
export function SettingsMembers() {
  const { slug } = useProjectContext();
  const project = useProjectStatus(slug).data?.project;
  const sharing = useSharing(slug);
  const toast = useToast();
  const [email, setEmail] = useState("");
  const [role, setRole] = useState<Role>("EDITOR");
  const owner = project?.role === "OWNER";
  useCrumbs([{ label: project?.name ?? slug, to: `/projects/${slug}` }, { label: "Settings", to: `/projects/${slug}/settings` }, { label: "Members" }]);
  const failure = (error: Error) => toast("Couldn’t update sharing", { body: error.message, tone: "critical" });
  const invite = useMutation({
    mutationFn: () => sharingApi.share(slug, email.trim(), role),
    onSuccess: async () => { setEmail(""); await sharingChanged(slug); toast("Project shared", { tone: "success" }); },
    onError: failure,
  });
  const change = useMutation({
    mutationFn: async ({ share, role }: { share: Share; role: Role }) => {
      if (share.subject) await api.setMember(slug, { subject: share.subject, display_name: share.display_name, email: share.email, role });
      else await sharingApi.share(slug, share.email ?? "", role);
    },
    onSuccess: () => sharingChanged(slug), onError: failure,
  });
  const remove = useMutation({ mutationFn: async (share: Share) => { await sharingApi.remove(slug, share); }, onSuccess: () => sharingChanged(slug), onError: failure });
  const busy = invite.isPending || change.isPending || remove.isPending;
  return <div className="page page-narrow">
    <div className="page-header"><div><h1 className="page-title">Members</h1><p className="page-subtitle">Choose who can access this project. Owners manage sharing, editors steer, and viewers read.</p></div></div>
    {sharing.isPending ? <Skeleton height={120} /> : sharing.isError ? <p role="alert" className="notice" data-tone="critical">{sharing.error.message}</p> : <div className="panel" style={{ marginBottom: "var(--space-5)" }}>
      {(sharing.data ?? []).map(share => <div className="row-link resource-row" key={share.subject ?? share.email}>
        <div style={{ minWidth: 0, flex: 1 }}><div style={{ fontWeight: 500 }}>{share.display_name}</div><div className="muted resource-value">{share.email}</div></div>
        {share.pending && <Tag tone="warning">Pending</Tag>}
        {owner ? <select className="select" aria-label={`Role for ${share.display_name}`} style={{ width: 130 }} value={share.role} disabled={busy} onChange={event => change.mutate({ share, role: event.target.value as Role })}>{roles.map(role => <option key={role} value={role}>{role.toLowerCase()}</option>)}</select> : <Tag>{share.role.toLowerCase()}</Tag>}
        {owner && <button type="button" className="btn btn-ghost btn-icon btn-sm" aria-label={`Remove ${share.display_name}`} disabled={busy} onClick={() => remove.mutate(share)}><TrashCan size={16} /></button>}
      </div>)}
    </div>}
    {owner && <form className="panel" onSubmit={event => { event.preventDefault(); invite.mutate(); }}>
      <div className="panel-header"><span className="panel-title">Share project</span></div>
      <div className="panel-body form-grid"><div className="form-row">
        <label className="field"><span className="field-label">Email</span><input className="input" type="email" required maxLength={320} autoComplete="email" placeholder="person@example.com" value={email} onChange={event => setEmail(event.target.value)} /></label>
        <label className="field"><span className="field-label">Role</span><select className="select" value={role} onChange={event => setRole(event.target.value as Role)}>{roles.map(role => <option key={role} value={role}>{role.toLowerCase()}</option>)}</select></label>
      </div></div>
      <div className="panel-footer"><span>The project appears when they sign in with this email. No email is sent.</span><button className="btn btn-primary btn-sm" disabled={busy || !email.trim()}><Add size={16} /> Share project</button></div>
    </form>}
  </div>;
}
