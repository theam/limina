import { Book, Chat, Close, Document, Settings } from "@carbon/icons-react";
import { useEffect, useRef, useState } from "react";
import { Link, Outlet, useLocation, useMatch, useNavigate } from "react-router";
import { Metrics } from "../../components/project/Metrics";
import { Summary } from "../../components/project/Summary";
import { Markdown } from "../../components/knowledge/Markdown";
import { useCrumbs } from "../../components/shell/Crumbs";
import { useProjectStatus } from "../../lib/queries";
import { Conversation } from "./Conversation";
import { useProjectContext } from "./ProjectContext";

/** Route changes affect only the context pane: the conversation and its draft stay mounted. */
export function Workspace() {
  const { slug } = useProjectContext();
  const project = useProjectStatus(slug).data?.project;
  const base = `/projects/${slug}`;
  const location = useLocation();
  const navigate = useNavigate();
  const [visible, setVisible] = useState(true);
  const [documents, setDocuments] = useState<string[]>([]);
  const [files, setFiles] = useState<string[]>([]);
  const toggle = useRef<HTMLButtonElement>(null);
  const context = useRef<HTMLElement>(null);
  const reveal = () => {
    setVisible(true);
    requestAnimationFrame(() => {
      if (window.matchMedia("(max-width: 740px)").matches)
        context.current?.scrollIntoView({ block: "start" });
    });
  };
  const suffix = location.pathname.slice(base.length);
  const key = useMatch("/projects/:slug/knowledge/:key")?.params.key ?? null;
  const file = suffix.startsWith("/results")
    ? new URLSearchParams(location.search).get("file")
    : null;
  const isDocument =
    key &&
    !["hypotheses", "experiments", "findings", "DASHBOARD"].includes(key);
  const section = suffix.startsWith("/knowledge")
    ? "knowledge"
    : suffix.startsWith("/results")
      ? "results"
      : "state";
  useCrumbs([
    { label: project?.name ?? slug, to: base },
    { label: section === "state" ? "Conversation" : section === "knowledge" ? "Knowledge" : "Results" },
    ...(isDocument ? [{ label: key }] : file ? [{ label: file }] : []),
  ]);
  useEffect(() => {
    setVisible(true);
    if (isDocument)
      setDocuments((current) =>
        current.includes(key) ? current : [...current.slice(-5), key],
      );
    if (file)
      setFiles((current) =>
        current.includes(file) ? current : [...current.slice(-2), file],
      );
  }, [location.key, isDocument, key, file]);

  useEffect(() => {
    if (
      (isDocument || file) &&
      window.matchMedia("(max-width: 740px)").matches
    ) {
      requestAnimationFrame(() =>
        context.current?.scrollIntoView({ block: "start" }),
      );
    }
  }, [isDocument, key, file]);

  const tabs = [
    { id: "state", label: "State", to: base },
    { id: "knowledge", label: "Knowledge", to: `${base}/knowledge` },
    { id: "results", label: "Results", to: `${base}/results` },
  ];
  return (
    <div className="project-workspace" data-context={visible}>
      <header className="workspace-heading">
        <div className="workspace-title">
          <span className="label">Project workspace</span>
          <h1>{project?.name ?? slug}</h1>
        </div>
        <div className="cluster workspace-actions">
          <Link className="btn btn-ghost btn-sm" to={`${base}/settings`}>
            <Settings size={14} aria-hidden="true" /> Settings
          </Link>
          <button
            ref={toggle}
            className="btn btn-sm"
            aria-expanded={visible}
            aria-controls="project-context"
            onClick={() => (visible ? setVisible(false) : reveal())}
          >
            <Book size={14} aria-hidden="true" />{" "}
            {visible ? "Hide context" : "Show context"}
          </button>
        </div>
      </header>
      <div className="workspace-body">
        <section
          className="workspace-conversation"
          aria-label="Conversation with Limina"
        >
          <Conversation />
        </section>
        <aside
          ref={context}
          id="project-context"
          className="workspace-context"
          aria-label="Project context"
          hidden={!visible}
        >
          <div className="workspace-context-heading">
            <span className="label">Project context</span>
            <button
              className="btn btn-ghost btn-icon btn-sm"
              aria-label="Close project context"
              onClick={() => {
                setVisible(false);
                toggle.current?.focus();
              }}
            >
              <Close size={16} />
            </button>
          </div>
          <nav className="workspace-tabs" aria-label="Context tabs">
            {tabs.map((tab) => (
              <Link
                key={tab.id}
                to={tab.to}
                aria-current={
                  !isDocument && !file && section === tab.id
                    ? "page"
                    : undefined
                }
                onClick={() => setVisible(true)}
              >
                {tab.label}
              </Link>
            ))}
            {documents.map((id) => (
              <span
                className="workspace-doc-tab"
                key={id}
                data-active={isDocument && key === id}
              >
                <Link
                  to={`${base}/knowledge/${encodeURIComponent(id)}`}
                  aria-current={isDocument && key === id ? "page" : undefined}
                >
                  <Document size={12} aria-hidden="true" />
                  {id}
                </Link>
                <button
                  aria-label={`Close ${id}`}
                  onClick={() => {
                    setDocuments((current) =>
                      current.filter((value) => value !== id),
                    );
                    if (key === id) navigate(`${base}/knowledge`);
                  }}
                >
                  <Close size={12} />
                </button>
              </span>
            ))}
            {files.map((name) => (
              <span
                className="workspace-doc-tab"
                key={`file:${name}`}
                data-active={file === name}
              >
                <Link
                  to={`${base}/results?file=${encodeURIComponent(name)}`}
                  aria-current={file === name ? "page" : undefined}
                >
                  <Document size={12} aria-hidden="true" />
                  {name}
                </Link>
                <button
                  aria-label={`Close ${name}`}
                  onClick={() => {
                    setFiles((current) =>
                      current.filter((value) => value !== name),
                    );
                    if (file === name) navigate(`${base}/results`);
                  }}
                >
                  <Close size={12} />
                </button>
              </span>
            ))}
          </nav>
          <div className="workspace-context-scroll">
            <Outlet />
          </div>
        </aside>
      </div>
    </div>
  );
}

export function WorkspaceState() {
  const { slug } = useProjectContext();
  const status = useProjectStatus(slug);
  const project = status.data?.project;
  if (!project) return <p className="muted">Loading project state…</p>;
  return (
    <div className="workspace-state">
      <section>
        <h2 className="section-title">The mission</h2>
        <p className="muted">
          Success is measured against your criteria and the evidence.
        </p>
        <Link className="link" to={`/projects/${slug}/knowledge/CHALLENGE`}>
          Read the full mission
        </Link>
      </section>
      <Metrics project={project} />
      <section>
        <Summary
          label="Current objective"
          source={project.current_objective}
          slug={slug}
        />
      </section>
      <section>
        <Summary label="Next step" source={project.next_step} slug={slug} />
      </section>
      <section>
        <h2 className="section-title">Success criteria</h2>
        <Markdown
          source={project.success_criteria || "No criteria recorded yet."}
          slug={slug}
        />
      </section>
      <Link className="section-link" to={`/projects/${slug}/knowledge/ACTIVE`}>
        <Chat size={14} aria-hidden="true" /> Read the working state
      </Link>
    </div>
  );
}
