import { Download } from "@carbon/icons-react";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { Link, Navigate, useParams } from "react-router";
import { Markdown, isArtifactKey } from "../../components/knowledge/Markdown";
import { Empty, Skeleton } from "../../components/ui/Empty";
import { ArtifactStatusTag } from "../../components/ui/Status";
import { Time } from "../../components/ui/Time";
import { api } from "../../lib/api";
import { keys, useKnowledgeDocument } from "../../lib/queries";
import { titleCase } from "../../lib/format";
import { useProjectContext } from "./ProjectContext";
import { KnowledgeHome } from "./KnowledgeHome";

/** Original revision fields are rendered in full; never reconstruct history from today's note. */
export function revisionMarkdown(content: Record<string, unknown>): string {
  return Object.entries(content)
    .filter(([, value]) => value !== null && value !== "")
    .map(([name, value]) => {
      const text =
        typeof value === "string" ? value : JSON.stringify(value, null, 2);
      return `## ${titleCase(name.replaceAll("_", " "))}\n\n${text}`;
    })
    .join("\n\n");
}

function saveBlob(blob: Blob, name: string) {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = name;
  anchor.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}
export { saveBlob };

function DocumentReader({ id }: { id: string }) {
  const { slug } = useProjectContext();
  const query = useKnowledgeDocument(slug, id);
  const [before, setBefore] = useState(2147483647);
  const history = useQuery({
    queryKey: [...keys.knowledge(slug), "revisions", id, before],
    queryFn: () => api.revisions(slug, id, before),
    enabled: isArtifactKey(id),
  });
  const [revision, setRevision] = useState<
    import("../../lib/types").Revision | undefined
  >();
  const selected = revision ? String(revision.version) : "current";
  const doc = query.vault?.resolve(id);
  if (query.isPending)
    return (
      <div className="stack">
        <Skeleton height={30} />
        <Skeleton height={120} />
      </div>
    );
  if (query.error || !doc)
    return (
      <Empty
        title="Couldn’t open this note"
        action={
          <button className="btn btn-sm" onClick={() => query.refetch()}>
            Try again
          </button>
        }
      >
        {query.error?.message ?? "This note is not available."}
      </Empty>
    );
  const source = revision ? revisionMarkdown(revision.content) : doc.body;
  const title = revision?.title ?? doc.title;
  return (
    <article className="workspace-document">
      <div className="cluster">
        <span className="id-chip">{id}</span>
        <span className="label">
          {revision ? "Historical version" : "Current note"}
        </span>
        <ArtifactStatusTag status={revision?.status ?? doc.status} />
      </div>
      <h2>{title}</h2>
      {!revision &&
        Object.entries(doc.properties)
          .filter(
            ([name, value]) =>
              ["Hypothesis", "Experiment"].includes(name) && value,
          )
          .map(([name, value]) => (
            <div className="workspace-note-parent" key={name}>
              <Markdown
                source={`**${name}** ${value}`}
                slug={slug}
                vault={query.vault}
              />
            </div>
          ))}
      {isArtifactKey(id) && (
        <div className="workspace-version">
          <label htmlFor="note-version">Note version</label>
          <select
            id="note-version"
            value={selected}
            onChange={(event) =>
              setRevision(
                history.data?.items.find(
                  (item) => String(item.version) === event.target.value,
                ),
              )
            }
          >
            <option value="current">Current version</option>
            {revision &&
              !history.data?.items.some(
                (item) => item.version === revision.version,
              ) && (
                <option value={revision.version}>
                  Selected version {revision.version}
                </option>
              )}
            {history.data?.items.map((item) => (
              <option key={item.version} value={item.version}>
                Version {item.version} ·{" "}
                {new Date(item.created_at).toLocaleString()}
              </option>
            ))}
          </select>
          <div className="cluster">
            {before !== 2147483647 && (
              <button
                className="btn btn-ghost btn-sm"
                onClick={() => setBefore(2147483647)}
              >
                Newest versions
              </button>
            )}
            {history.data?.next_before != null && (
              <button
                className="btn btn-ghost btn-sm"
                onClick={() => setBefore(history.data!.next_before!)}
              >
                Older versions
              </button>
            )}
          </div>
          {history.isPending && (
            <p className="muted" role="status">
              Loading history…
            </p>
          )}
          {history.error && (
            <p className="muted">
              History is unavailable.{" "}
              <button className="link" onClick={() => history.refetch()}>
                Try again
              </button>
            </p>
          )}
        </div>
      )}
      {revision && (
        <p className="workspace-history-note" role="status">
          Read-only version from <Time iso={revision.created_at} />. Viewing it
          does not change the current project state.
        </p>
      )}
      <Markdown
        source={source || "No content recorded yet."}
        slug={slug}
        vault={revision ? undefined : query.vault}
        headingLevel={3}
        autolink
      />
      <div className="cluster workspace-document-actions">
        <button
          className="btn btn-sm"
          onClick={() =>
            saveBlob(
              new Blob([`# ${title}\n\n${source}`], { type: "text/markdown" }),
              `${id}${revision ? `-v${revision.version}` : ""}.md`,
            )
          }
        >
          <Download size={14} aria-hidden="true" /> Download note
        </button>
        {/^[HE]\d/.test(id) && (
          <Link
            className="link"
            to={`/projects/${slug}/knowledge?parent=${encodeURIComponent(id)}`}
          >
            Explore related evidence
          </Link>
        )}
      </div>
    </article>
  );
}

export function WorkspaceDocument() {
  const id = useParams().key!;
  const { slug } = useProjectContext();
  if (id.toUpperCase() === "DASHBOARD")
    return <Navigate to={`/projects/${slug}`} replace />;
  const kind = { hypotheses: "H", experiments: "E", findings: "F" }[
    id.toLowerCase()
  ];
  return kind ? (
    <KnowledgeHome kind={kind} embedded />
  ) : (
    <DocumentReader key={id} id={id} />
  );
}
