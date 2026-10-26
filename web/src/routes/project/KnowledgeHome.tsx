import { ArrowLeft, ArrowRight, Book, Search } from "@carbon/icons-react";
import { Link, useLocation, useSearchParams } from "react-router";
import { Empty, Skeleton } from "../../components/ui/Empty";
import { ArtifactStatusTag } from "../../components/ui/Status";
import { Time } from "../../components/ui/Time";
import { artifactPreview, KNOWLEDGE_KINDS } from "../../lib/knowledge";
import { useKnowledgeList, useProjectStatus } from "../../lib/queries";
import { useDebouncedValue } from "../../lib/useDebouncedValue";
import type { Artifact } from "../../lib/types";
import { titleCase } from "../../lib/format";
import { useProjectContext } from "./ProjectContext";

const PAGE_SIZE = 25;

export function KnowledgeRow({ artifact, slug }: { artifact: Artifact; slug: string }) {
  const preview = artifactPreview(artifact);
  return (
    <article className="knowledge-result">
      <Link to={`/projects/${slug}/knowledge/${artifact.id}`} className="knowledge-result-main">
        <div className="cluster knowledge-result-heading">
          <span className="id-chip">{artifact.id}</span>
          <h2>{artifact.title}</h2>
          <ArtifactStatusTag status={artifact.status} />
        </div>
        {preview && <p className="muted">{preview}</p>}
        <span className="section-link">Read note <ArrowRight size={14} aria-hidden="true" /></span>
      </Link>
      <footer className="knowledge-result-meta">
        <div className="cluster">
          {artifact.hypothesis_id && <Link className="link" to={`/projects/${slug}/knowledge/${artifact.hypothesis_id}`}>Hypothesis {artifact.hypothesis_id}</Link>}
          {artifact.experiment_id && <Link className="link" to={`/projects/${slug}/knowledge/${artifact.experiment_id}`}>Experiment {artifact.experiment_id}</Link>}
        </div>
        <span className="muted">Updated <Time iso={artifact.updated_at} /></span>
      </footer>
    </article>
  );
}

/** Search and navigation stay bounded even when a project has thousands of artifacts. */
export function KnowledgeHome({ kind, embedded = false }: { kind?: string; embedded?: boolean } = {}) {
  const { slug } = useProjectContext();
  const status = useProjectStatus(slug);
  const [params, setParams] = useSearchParams();
  const location = useLocation();
  const query = params.get("q") ?? "";
  const settled = useDebouncedValue(query);
  const state = params.get("status") ?? "";
  const parent = params.get("parent") ?? "";
  const cursor = params.get("cursor") ?? "";
  const category = KNOWLEDGE_KINDS.find((item) => item.value === kind);
  const results = useKnowledgeList(slug, {
    query: settled || undefined, kind, status: state || undefined,
    parent: parent || undefined, cursor: cursor || undefined, limit: PAGE_SIZE,
  }, settled === query);
  const title = category?.label ?? "Knowledge";
  const Heading = embedded ? "h2" : "h1";
  const statuses = [...new Set(Object.entries(status.data?.knowledge ?? {})
    .filter(([key]) => !kind || key === kind).flatMap(([, counts]) => Object.keys(counts)))];
  const searching = query !== settled || results.isPending;
  const base = `/projects/${slug}/knowledge`;
  const change = (key: string, value: string) => {
    setParams((current) => {
      const next = new URLSearchParams(current);
      if (value) next.set(key, value); else next.delete(key);
      next.delete("cursor");
      return next;
    }, { replace: key === "q", state: null });
  };
  const categoryLink = (path: string) => {
    const next = new URLSearchParams(params);
    next.delete("cursor"); next.delete("status");
    return `${path}${next.size ? `?${next}` : ""}`;
  };
  const pageLink = (nextCursor?: string) => {
    const next = new URLSearchParams(params);
    if (nextCursor) next.set("cursor", nextCursor); else next.delete("cursor");
    return `${location.pathname}?${next}`;
  };
  const previous = typeof location.state?.previous === "string" ? location.state.previous : undefined;
  const filtered = Boolean(query || state || parent);
  const total = results.data?.total ?? 0;
  const noteLabel = total === 1 ? "note" : "notes";

  return (
    <div className="page knowledge-browser">
      <div className="page-header">
        <div>
          <Heading className="page-title">{title}</Heading>
          <p className="page-subtitle">Find evidence, read a note, and follow its research chain.</p>
        </div>
        <div className="page-actions">
          <Link className="btn" to={`${base}/CHALLENGE`}>Read mission</Link>
          <Link className="btn" to={`${base}/ACTIVE`}>Current state</Link>
        </div>
      </div>

      <nav className="knowledge-categories" aria-label="Knowledge categories">
        <Link to={categoryLink(base)} aria-current={!kind ? "page" : undefined}>
          <span>All research</span><small>Search across the project</small>
        </Link>
        {KNOWLEDGE_KINDS.map((item) => (
          <Link key={item.value} to={categoryLink(`${base}/${item.path}`)} aria-current={kind === item.value ? "page" : undefined}>
            <span>{item.label}</span><small>{item.description}</small>
          </Link>
        ))}
      </nav>

      <div className="knowledge-filters">
        <label className="search-field">
          <Search size={16} aria-hidden="true" />
          <span className="sr-only">Search project knowledge</span>
          <input className="input" name="knowledge-search" type="search" autoComplete="off"
            placeholder="Search by ID, title or content…" maxLength={500} value={query}
            onChange={(event) => change("q", event.target.value)} />
        </label>
        <label className="field knowledge-status-filter">
          <span className="sr-only">Filter by status</span>
          <select className="select" value={state} onChange={(event) => change("status", event.target.value)}>
            <option value="">All statuses</option>
            {state && !statuses.includes(state) && <option value={state}>{titleCase(state)}</option>}
            {statuses.map((item) => <option key={item} value={item}>{titleCase(item)}</option>)}
          </select>
        </label>
        {filtered && <button className="btn btn-ghost" onClick={() => setParams({})}>Clear filters</button>}
      </div>
      {parent && <div className="knowledge-scope">Evidence for <Link className="link" to={`${base}/${parent}`}>{parent}</Link><button className="btn btn-ghost btn-sm" onClick={() => change("parent", "")}>Search all research</button></div>}
      <p className="field-hint knowledge-result-count" role="status" aria-live="polite">
        {searching ? "Searching project knowledge…" : results.isError ? "Search unavailable." : `${total} ${query ? "matching " : ""}${noteLabel}. Most recently updated first.`}
      </p>

      {searching ? <div className="stack"><Skeleton height={140} /><Skeleton height={140} /></div> : results.isError ? (
        <Empty title="Couldn’t load knowledge" action={<button className="btn" onClick={() => results.refetch()}>Try again</button>}>
          {(results.error as Error).message}
        </Empty>
      ) : results.data?.items.length ? (
        <>
          <div className="panel knowledge-results">{results.data.items.map((artifact) => <KnowledgeRow key={artifact.id} artifact={artifact} slug={slug} />)}</div>
          <nav className="pagination" aria-label="Knowledge pages">
            {cursor ? <Link className="btn" to={previous ?? pageLink()}><ArrowLeft size={14} aria-hidden="true" />{previous ? "Previous page" : "First page"}</Link> : <span />}
            <span className="muted">{results.data.items.length} {results.data.items.length === 1 ? "note" : "notes"} on this page</span>
            {results.data.next_cursor ? <Link className="btn" to={pageLink(results.data.next_cursor)} state={{ previous: `${location.pathname}${location.search}` }}>Next page <ArrowRight size={14} aria-hidden="true" /></Link> : <span />}
          </nav>
        </>
      ) : <Empty icon={<Book size={20} aria-hidden="true" />} title={filtered ? "No notes match these filters" : "No research notes yet"}>
        {filtered ? "Try another term or clear the filters. Search includes the entire project, not just this page." : "Hypotheses, experiments and findings will appear here as evidence is recorded."}
      </Empty>}
    </div>
  );
}
