import { ArrowLeft, ArrowRight, DocumentBlank } from "@carbon/icons-react";
import { Fragment, useEffect, useMemo, useState } from "react";
import { Link, Navigate, useLocation, useParams } from "react-router";
import { useCrumbs } from "../../components/shell/Crumbs";
import { Markdown, WikiLink, isArtifactKey } from "../../components/knowledge/Markdown";
import { Empty, Skeleton } from "../../components/ui/Empty";
import { ArtifactStatusTag, Tag } from "../../components/ui/Status";
import { useKnowledgeDocument, useProjectStatus } from "../../lib/queries";
import { ApiError } from "../../lib/api";
import { KnowledgeHome } from "./KnowledgeHome";
import { GROUP_LABELS, type DocGroup, type Vault, type VaultDoc } from "../../lib/vault";
import { useProjectContext } from "./ProjectContext";

const GROUPS: Record<string, { group: DocGroup; kind: VaultDoc["kind"]; singular: string; blurb: string }> = {
  hypotheses: {
    group: "hypotheses",
    kind: "hypothesis",
    singular: "Hypothesis",
    blurb: "Falsifiable claims about what should work and why.",
  },
  experiments: {
    group: "experiments",
    kind: "experiment",
    singular: "Experiment",
    blurb: "Decisive tests of a hypothesis, with baselines, guardrails and observations.",
  },
  findings: {
    group: "findings",
    kind: "finding",
    singular: "Finding",
    blurb: "What the evidence actually established, and the debt that remains.",
  },
};

const KIND_LABEL: Record<VaultDoc["kind"], string> = {
  hypothesis: "Hypothesis",
  experiment: "Experiment",
  finding: "Finding",
  mission: "Mission",
  active: "Active state",
  dashboard: "Dashboard",
  note: "Note",
};

/* ─── Document ──────────────────────────────────────────────────────── */

function PropertyValue({ value, vault, slug }: { value: string; vault: Vault; slug: string }) {
  const parts = value.split(/(\[\[[^\]]+\]\])/g).filter(Boolean);
  return (
    <>
      {parts.map((part, index) => {
        const match = part.match(/^\[\[([^\]|]+)(?:\|[^\]]+)?\]\]$/);
        const doc = match ? vault.resolve(match[1]) : undefined;
        if (doc) return <WikiLink key={index} doc={doc} slug={slug} showTitle />;
        if (match) return <Link key={index} className="link" to={`/projects/${slug}/knowledge/${encodeURIComponent(match[1])}`}>{match[1]}</Link>;
        if (/^\d{4}-\d{2}-\d{2}/.test(part)) {
          return <Fragment key={index}>{new Date(part).toLocaleDateString(undefined, { dateStyle: "medium" })}</Fragment>;
        }
        return <Fragment key={index}>{part}</Fragment>;
      })}
    </>
  );
}

function Properties({ doc, vault, slug }: { doc: VaultDoc; vault: Vault; slug: string }) {
  const entries = Object.entries(doc.properties).filter(([name, value]) => value && name !== "Status");
  if (!doc.status && !entries.length) return null;
  return (
    <dl className="properties">
      {doc.status && doc.kind !== "finding" && (
        <>
          <dt>Status</dt>
          <dd>
            <ArtifactStatusTag status={doc.status} />
          </dd>
        </>
      )}
      {entries.map(([name, value]) => (
        <Fragment key={name}>
          <dt>{name}</dt>
          <dd>
            {name === "Impact" ? (
              <Tag tone={value === "CRITICAL" ? "critical" : value === "HIGH" ? "warning" : "neutral"}>
                {value.toLowerCase()}
              </Tag>
            ) : (
              <PropertyValue value={value} vault={vault} slug={slug} />
            )}
          </dd>
        </Fragment>
      ))}
    </dl>
  );
}

function DocLinkList({ docs, slug }: { docs: { doc: VaultDoc; label?: string }[]; slug: string }) {
  return (
    <ul className="doc-links">
      {docs.map(({ doc, label }) => (
        <li key={`${label}-${doc.key}`}>
          <Link to={`/projects/${slug}/knowledge/${doc.key}`} className="doc-link">
            {label && <span className="doc-link-label">{label}</span>}
            {isArtifactKey(doc.key) && <span className="id-chip">{doc.key}</span>}
            <span className="doc-link-title">{doc.title}</span>
          </Link>
        </li>
      ))}
    </ul>
  );
}

function useActiveHeading(ids: string[]): string | null {
  const [active, setActive] = useState<string | null>(null);
  useEffect(() => {
    if (!ids.length) return;
    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries.filter((entry) => entry.isIntersecting).sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
        if (visible[0]) setActive(visible[0].target.id);
      },
      { rootMargin: "-64px 0px -65% 0px" },
    );
    ids.forEach((id) => {
      const element = document.getElementById(id);
      if (element) observer.observe(element);
    });
    return () => observer.disconnect();
  }, [ids]);
  return active;
}

function Doc({ doc, vault }: { doc: VaultDoc; vault: Vault }) {
  const { slug } = useProjectContext();
  const { hash } = useLocation();
  // Every research note links to Mission/Active/Dashboard; that boilerplate is noise when reading.
  const research = doc.group !== "project";
  const boilerplate = (key: string) => research && ["CHALLENGE", "ACTIVE", "DASHBOARD"].includes(key);
  const backlinks = vault.backlinks(doc.key).filter((item) => item.key !== "DASHBOARD");
  const outgoing = doc.connections
    .map((connection) => ({ doc: vault.resolve(connection.target), label: connection.label }))
    .filter(
      (item): item is { doc: VaultDoc; label: string } =>
        Boolean(item.doc) && item.doc!.key !== doc.key && !boilerplate(item.doc!.key),
    );
  const siblings = vault.docs.filter((item) => item.group === doc.group && item.kind === doc.kind);
  const index = siblings.findIndex((item) => item.key === doc.key);
  const previous = index > 0 ? siblings[index - 1] : undefined;
  const next = index >= 0 && index < siblings.length - 1 ? siblings[index + 1] : undefined;
  const headingIds = useMemo(() => doc.outline.map((item) => item.id), [doc.outline]);
  const activeHeading = useActiveHeading(headingIds);

  useEffect(() => {
    if (hash) document.getElementById(decodeURIComponent(hash.slice(1)))?.scrollIntoView();
    else window.scrollTo({ top: 0 });
  }, [doc.key, hash]);

  return (
    <div className="page doc-layout">
      <article className="doc">
        <div className="doc-eyebrow">
          <span className="label">{KIND_LABEL[doc.kind]}</span>
          {isArtifactKey(doc.key) && <span className="id-chip">{doc.key}</span>}
        </div>
        <h1 className="doc-title">{doc.title}</h1>
        <Properties doc={doc} vault={vault} slug={slug} />
        {doc.body.trim() ? (
          <Markdown source={doc.body} slug={slug} vault={vault} autolink className="doc-body" />
        ) : (
          <p className="muted doc-body">This note has no content yet.</p>
        )}
        {doc.pending.length > 0 && (
          <p className="doc-pending">
            <span className="label">Not recorded yet</span> {doc.pending.join(" · ")}
          </p>
        )}

        {(doc.kind === "hypothesis" || doc.kind === "experiment") && (
          <section className="section">
            <h2 className="section-title">Follow the evidence</h2>
            <p className="muted">Explore the experiments and findings connected to this note, one page at a time.</p>
            <Link className="btn" to={`/projects/${slug}/knowledge?parent=${encodeURIComponent(doc.key)}`}>
              Explore related evidence <ArrowRight size={14} aria-hidden="true" />
            </Link>
          </section>
        )}
        {(outgoing.length > 0 || backlinks.length > 0) && (
          <section className="connections" id="connections">
            {outgoing.length > 0 && (
              <div>
                <h2 className="label">Links</h2>
                <DocLinkList docs={outgoing} slug={slug} />
              </div>
            )}
            {backlinks.length > 0 && (
              <div>
                <h2 className="label">Referenced by</h2>
                <DocLinkList docs={backlinks.map((item) => ({ doc: item }))} slug={slug} />
              </div>
            )}
          </section>
        )}

        {(previous || next) && (
          <nav className="doc-pager" aria-label="Adjacent notes">
            {previous ? (
              <Link to={`/projects/${slug}/knowledge/${previous.key}`} className="doc-pager-link">
                <span className="label">
                  <ArrowLeft size={12} /> Previous
                </span>
                <span>
                  {isArtifactKey(previous.key) && <span className="id-chip">{previous.key}</span>} {previous.title}
                </span>
              </Link>
            ) : (
              <span />
            )}
            {next && (
              <Link to={`/projects/${slug}/knowledge/${next.key}`} className="doc-pager-link" data-align="end">
                <span className="label">
                  Next <ArrowRight size={12} />
                </span>
                <span>
                  {isArtifactKey(next.key) && <span className="id-chip">{next.key}</span>} {next.title}
                </span>
              </Link>
            )}
          </nav>
        )}
      </article>

      <aside className="doc-rail" aria-label="On this page">
        {doc.outline.length > 1 && (
          <div>
            <div className="label">On this page</div>
            <ul className="outline">
              {doc.outline.map((item) => (
                <li key={item.id} data-depth={item.depth}>
                  <a href={`#${item.id}`} aria-current={activeHeading === item.id ? "true" : undefined}>
                    {item.text}
                  </a>
                </li>
              ))}
            </ul>
          </div>
        )}
        {backlinks.length > 0 && (
          <div>
            <div className="label">Referenced by {backlinks.length}</div>
            <ul className="outline">
              {backlinks.slice(0, 8).map((item) => (
                <li key={item.key}>
                  <Link to={`/projects/${slug}/knowledge/${item.key}`}>
                    {isArtifactKey(item.key) ? `${item.key} · ` : ""}
                    {item.title}
                  </Link>
                </li>
              ))}
            </ul>
          </div>
        )}
      </aside>
    </div>
  );
}

function KnowledgeDocument() {
  const { slug } = useProjectContext();
  const key = useParams().key!;
  const status = useProjectStatus(slug);
  const { vault, isPending, error, refetch } = useKnowledgeDocument(slug, key);
  const group = GROUPS[key.toLowerCase()];
  const doc = !group ? vault?.resolve(key) : undefined;
  const projectCrumb = { label: status.data?.project.name ?? slug, to: `/projects/${slug}` };
  const knowledgeCrumb = { label: "Knowledge", to: `/projects/${slug}/knowledge` };
  const docGroup = doc && doc.group !== "project" ? doc.group : null;

  useCrumbs(
    group
      ? [projectCrumb, knowledgeCrumb, { label: GROUP_LABELS[group.group] }]
      : [
          projectCrumb,
          knowledgeCrumb,
          ...(docGroup ? [{ label: GROUP_LABELS[docGroup], to: `/projects/${slug}/knowledge/${docGroup}` }] : []),
          { label: doc ? (isArtifactKey(doc.key) ? doc.key : doc.title) : key },
        ],
  );

  if (isPending) {
    return (
      <div className="page doc-layout">
        <div className="doc stack">
          <Skeleton width={120} height={14} />
          <Skeleton width="70%" height={34} />
          <Skeleton height={16} />
          <Skeleton height={16} />
          <Skeleton width="80%" height={16} />
        </div>
      </div>
    );
  }
  if (error instanceof ApiError && error.status === 404) return (
    <div className="page page-narrow"><Empty title="Note not found" action={<Link className="btn" to={`/projects/${slug}/knowledge`}>Search project knowledge</Link>}>
      This note may not have been recorded yet. Search the project for related evidence.
    </Empty></div>
  );
  if (error || !vault) {
    return (
      <div className="page page-narrow">
        <Empty title="Couldn’t load this note" action={<button className="btn" onClick={() => refetch()}>Try again</button>}>{(error as Error | null)?.message}</Empty>
      </div>
    );
  }

  if (!doc) {
    return (
      <div className="page page-narrow">
        <Empty
          icon={<DocumentBlank size={20} />}
          title={`“${key}” isn’t in this vault`}
          action={
            <Link className="btn" to={`/projects/${slug}/knowledge`}>
              Back to knowledge
            </Link>
          }
        >
          The link may point to a note that hasn’t been written yet.
        </Empty>
      </div>
    );
  }
  return <Doc key={doc.key} doc={doc} vault={vault} />;
}


export function KnowledgePage() {
  const key = useParams().key!;
  const kind = { hypotheses: "H", experiments: "E", findings: "F" }[key.toLowerCase()];
  if (kind) return <KnowledgeHome kind={kind} />;
  if (key.toUpperCase() === "DASHBOARD") return <Navigate to=".." replace />;
  return <KnowledgeDocument />;
}
