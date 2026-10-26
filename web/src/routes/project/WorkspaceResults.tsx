import { Document, Download } from "@carbon/icons-react";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { Link, useSearchParams } from "react-router";
import { Markdown } from "../../components/knowledge/Markdown";
import { Empty, Skeleton } from "../../components/ui/Empty";
import { useToast } from "../../components/ui/Toast";
import { api } from "../../lib/api";
import { keys } from "../../lib/queries";
import { saveBlob } from "./WorkspaceDocument";
import { useProjectContext } from "./ProjectContext";

function OutputReader({ name }: { name: string }) {
  const { slug } = useProjectContext();
  const query = useQuery({
    queryKey: [...keys.outputs(slug), name],
    queryFn: () => api.outputPreview(slug, name),
  });
  const [downloading, setDownloading] = useState(false);
  const toast = useToast();
  const download = async () => {
    setDownloading(true);
    try {
      saveBlob(await api.outputDownload(slug, name), name);
    } catch (error) {
      toast("Couldn’t download this file", {
        body: (error as Error).message,
        tone: "critical",
      });
    } finally {
      setDownloading(false);
    }
  };
  return (
    <article className="workspace-document">
      <Link className="section-link" to={`/projects/${slug}/results`}>
        ← All results
      </Link>
      <h2>{name}</h2>
      <button
        className="btn btn-sm"
        disabled={downloading}
        onClick={() => void download()}
      >
        <Download size={14} aria-hidden="true" />
        {downloading ? "Downloading…" : "Download file"}
      </button>
      {query.isPending ? (
        <Skeleton height={160} />
      ) : query.error ? (
        <Empty
          title="Couldn’t preview this file"
          action={
            <button className="btn btn-sm" onClick={() => query.refetch()}>
              Try again
            </button>
          }
        >
          {query.error.message}
        </Empty>
      ) : (
        query.data && (
          <>
            {query.data.truncated && (
              <p className="workspace-history-note">
                Preview limited to 128 KiB. Download the file to read it in
                full.
              </p>
            )}
            {query.data.text === null ? (
              <p className="muted">
                Download this file to open it in a compatible app.
              </p>
            ) : /\.md$/i.test(name) ? (
              <Markdown
                source={query.data.text}
                slug={slug}
                headingLevel={3}
                autolink
                publishedOutput
              />
            ) : (
              <pre className="workspace-file-text">{query.data.text}</pre>
            )}
          </>
        )
      )}
    </article>
  );
}

export function WorkspaceResults() {
  const { slug } = useProjectContext();
  const [params, setParams] = useSearchParams();
  const selected = params.get("file");
  const offset = Math.max(0, Math.min(1000, Number(params.get("offset")) || 0));
  const query = useQuery({
    queryKey: [...keys.outputs(slug), { offset }],
    queryFn: () => api.outputs(slug, offset),
    enabled: !selected,
  });
  if (selected) return <OutputReader key={selected} name={selected} />;
  return (
    <div className="workspace-results">
      <h2>Results</h2>
      <p className="muted">
        Deliverables published by the agent. Open a file here or download it to
        keep working.
      </p>
      {query.isPending ? (
        <Skeleton height={150} />
      ) : query.error ? (
        <Empty
          title="Couldn’t load results"
          action={
            <button className="btn btn-sm" onClick={() => query.refetch()}>
              Try again
            </button>
          }
        >
          {query.error.message}
        </Empty>
      ) : !query.data?.items.length ? (
        <Empty title="No published files yet">
          Ask Limina to prepare a report or export. Research evidence is
          available in Knowledge.
        </Empty>
      ) : (
        <ul className="workspace-files">
          {query.data.items.map((file) => (
            <li key={file.name}>
              <Link to={`?file=${encodeURIComponent(file.name)}`}>
                <Document size={18} aria-hidden="true" />
                <span>
                  <strong>{file.name}</strong>
                  <small>
                    {new Intl.NumberFormat(undefined, {
                      maximumFractionDigits: 1,
                    }).format(file.bytes / 1024)}{" "}
                    KiB · Open file
                  </small>
                </span>
              </Link>
            </li>
          ))}
        </ul>
      )}
      {query.data?.truncated && (
        <p className="muted">
          Showing files from the first 1,000 directory entries. Ask the agent to
          consolidate older deliverables.
        </p>
      )}
      <nav className="cluster" aria-label="Result pages">
        {offset > 0 && (
          <button
            className="btn btn-sm"
            onClick={() =>
              setParams({ offset: String(Math.max(0, offset - 25)) })
            }
          >
            Previous
          </button>
        )}
        {query.data?.next_offset != null && (
          <button
            className="btn btn-sm"
            onClick={() =>
              setParams({ offset: String(query.data!.next_offset) })
            }
          >
            Next
          </button>
        )}
      </nav>
    </div>
  );
}
