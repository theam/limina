import { ArrowUpRight, Link as LinkIcon } from "@carbon/icons-react";
import * as HoverCard from "@radix-ui/react-hover-card";
import { Children, isValidElement, memo, useMemo } from "react";
import ReactMarkdown, { defaultUrlTransform, type Components } from "react-markdown";
import { Link } from "react-router";
import remarkGfm from "remark-gfm";
import { headingId, linkifyWikilinks, type Vault, type VaultDoc } from "../../lib/vault";
import { ArtifactStatusTag } from "../ui/Status";

const KIND_LABEL: Record<string, string> = {
  hypothesis: "Hypothesis",
  experiment: "Experiment",
  finding: "Finding",
  mission: "Mission",
  active: "Active state",
  dashboard: "Dashboard",
  note: "Note",
};

export const isArtifactKey = (key: string) => /^(H|E|F|L|CR|SR)\d{3,}$/.test(key);

/** Resolve local references without loading a vault or exposing arbitrary workspace files. */
function projectReference(href: string, slug: string, publishedOutput: boolean): string | null {
  if (/^(?:[a-z][a-z0-9+.-]*:|\/\/|#)/i.test(href)) return null;
  const hashAt = href.indexOf("#");
  const hash = hashAt < 0 ? "" : href.slice(hashAt);
  let path: string;
  try {
    path = decodeURIComponent(hashAt < 0 ? href : href.slice(0, hashAt));
  } catch {
    return null;
  }
  const name = path.split("/").at(-1) ?? "";
  const explicitOutput = /(?:^|\/)outputs\/[^/]+$/.test(path);
  const note = name.match(/^((?:H|E|F|L|CR|SR)\d{3,})(?:[-_][^/]*)?\.md$/i)?.[1]
    ?? name.match(/^(CHALLENGE|ACTIVE|DASHBOARD)\.md$/i)?.[1];
  if (note && !explicitOutput) return `/projects/${slug}/knowledge/${note.toUpperCase()}${hash}`;
  const output = explicitOutput
    || (publishedOutput && /^\.?\/?[^/]+$/.test(path));
  if (output && name && !name.startsWith(".") && !name.includes("\\")) {
    return `/projects/${slug}/results?file=${encodeURIComponent(name)}${hash}`;
  }
  return href.startsWith(`/projects/${slug}/`) ? href : null;
}

function textOf(children: React.ReactNode): string {
  return Children.toArray(children)
    .map((child) =>
      typeof child === "string" || typeof child === "number"
        ? String(child)
        : isValidElement<{ children?: React.ReactNode }>(child)
          ? textOf(child.props.children)
          : "",
    )
    .join("");
}

export function DocPreview({ doc }: { doc: VaultDoc }) {
  return (
    <div className="doc-preview">
      <div className="cluster" style={{ gap: 6 }}>
        <span className="label">{KIND_LABEL[doc.kind]}</span>
        {isArtifactKey(doc.key) && <span className="id-chip">{doc.key}</span>}
        <span className="spacer" />
        <ArtifactStatusTag status={doc.kind === "finding" ? null : doc.status} />
      </div>
      <div className="doc-preview-title">{doc.title}</div>
      {doc.excerpt && <p className="doc-preview-excerpt">{doc.excerpt}</p>}
    </div>
  );
}

export function WikiLink({
  doc,
  slug,
  children,
  showTitle,
  compact,
}: {
  doc: VaultDoc;
  slug: string;
  children?: React.ReactNode;
  showTitle?: boolean;
  compact?: boolean;
}) {
  return (
    <HoverCard.Root openDelay={300} closeDelay={80}>
      <HoverCard.Trigger asChild>
        <Link to={`/projects/${slug}/knowledge/${doc.key}`} className={compact ? "wikilink-ref" : "wikilink"}>
          {showTitle ? (
            <>
              {isArtifactKey(doc.key) && <span className="wikilink-id">{doc.key}</span>}
              <span className="wikilink-title">{doc.title}</span>
            </>
          ) : (
            children
          )}
        </Link>
      </HoverCard.Trigger>
      <HoverCard.Portal>
        <HoverCard.Content className="floating hover-card" side="bottom" align="start" sideOffset={6}>
          <DocPreview doc={doc} />
        </HoverCard.Content>
      </HoverCard.Portal>
    </HoverCard.Root>
  );
}

/** Link syntactically valid artifact IDs (H001, E002…), including notes not yet loaded, outside code spans. */
function autolinkIds(markdown: string): string {
  const parts = markdown.split(/(```[\s\S]*?```|`[^`\n]*`|\[[^\]]*\]\([^)]*\)|\[\[[^\]]*\]\])/g);
  return parts
    .map((part, index) =>
      index % 2 === 1
        ? part
        : part.replace(/\b((?:H|E|F|L|CR|SR)\d{3,})\b/g, (match) => `[[${match}]]`),
    )
    .join("");
}

function Heading({ level, children }: { level: 2 | 3 | 4 | 5 | 6; children?: React.ReactNode }) {
  const id = headingId(textOf(children));
  const Tag = `h${level}` as "h2";
  return (
    <Tag id={id} className="heading">
      {children}
      <a href={`#${id}`} className="heading-anchor" aria-label="Link to this section">
        <LinkIcon size={14} />
      </a>
    </Tag>
  );
}

interface MarkdownProps {
  source: string;
  slug: string;
  vault?: Vault;
  autolink?: boolean;
  className?: string;
  headingLevel?: 2 | 3;
  publishedOutput?: boolean;
}

export const Markdown = memo(function Markdown({ source, slug, vault, autolink, className, headingLevel = 2, publishedOutput = false }: MarkdownProps) {
  const content = useMemo(() => {
    const linked = autolink ? autolinkIds(source) : source;
    return linkifyWikilinks(linked);
  }, [source, vault, autolink]);

  const components = useMemo<Components>(
    () => ({
      a({ href, children }) {
        if (href?.startsWith("wiki:")) {
          const target = decodeURIComponent(href.slice(5));
          const doc = vault?.resolve(target);
          if (!doc && slug && (isArtifactKey(target) || ["CHALLENGE", "ACTIVE", "DASHBOARD"].includes(target.toUpperCase()))) {
            return <Link className="wikilink" to={`/projects/${slug}/knowledge/${encodeURIComponent(target)}`}>{children}</Link>;
          }
          if (!doc) {
            return (
              <span className="wikilink-missing" title="This note doesn’t exist yet">
                {children}
              </span>
            );
          }
          const label = textOf(children);
          const bare = label.toLowerCase() === target.toLowerCase();
          // Inline references stay compact (ID only, title on hover) so prose keeps reading
          // naturally; named notes like [[ACTIVE]] show their human title instead of the alias.
          if (bare && isArtifactKey(doc.key)) {
            return (
              <WikiLink doc={doc} slug={slug} compact>
                {doc.key}
              </WikiLink>
            );
          }
          return (
            <WikiLink doc={doc} slug={slug}>
              {bare ? doc.title : children}
            </WikiLink>
          );
        }
        if (href?.startsWith("#")) return <a href={href}>{children}</a>;
        if (!href) return <span>{children}</span>;
        const projectLink = projectReference(href, slug, publishedOutput);
        if (projectLink) return <Link to={projectLink}>{children}</Link>;
        if (!/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i.test(href)) {
          return <span>{children}<span className="muted"> (workspace file; ask Limina to publish it)</span></span>;
        }
        return (
          <a href={href} target="_blank" rel="noreferrer" className="external">
            {children}
            <ArrowUpRight size={12} className="external-icon" />
          </a>
        );
      },
      h1: ({ children }) => <Heading level={headingLevel}>{children}</Heading>,
      h2: ({ children }) => <Heading level={headingLevel}>{children}</Heading>,
      h3: ({ children }) => <Heading level={headingLevel === 3 ? 4 : 3}>{children}</Heading>,
      h4: ({ children }) => <Heading level={headingLevel === 3 ? 5 : 4}>{children}</Heading>,
      table: ({ children }) => (
        <div className="table-scroll">
          <table>{children}</table>
        </div>
      ),
    }),
    [vault, slug, headingLevel, publishedOutput],
  );

  return (
    <div className={`prose${className ? ` ${className}` : ""}`}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={components}
        urlTransform={(url) => (url.startsWith("wiki:") ? url : defaultUrlTransform(url))}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
});
