/**
 * Parse Limina's Obsidian-compatible Markdown projection into a navigable vault.
 *
 * The runtime exports one Markdown file per note with YAML frontmatter (aliases, type, status),
 * Obsidian wikilinks (`[[H001]]`), a leading metadata blockquote, and a `## Links` section.
 * The reader renders frontmatter as properties, resolves wikilinks through aliases, computes
 * backlinks, and hides projection boilerplate that the UI already presents structurally.
 */

export type DocKind = "mission" | "active" | "dashboard" | "hypothesis" | "experiment" | "finding" | "note";
export type DocGroup = "hypotheses" | "experiments" | "findings" | "project";

export interface Connection {
  label: string;
  target: string;
}

export interface Heading {
  depth: 2 | 3;
  text: string;
  id: string;
}

export interface VaultDoc {
  /** Canonical route key, e.g. "H001", "CHALLENGE", "ACTIVE". */
  key: string;
  path: string;
  kind: DocKind;
  group: DocGroup;
  title: string;
  aliases: string[];
  status: string | null;
  properties: Record<string, string>;
  /** Markdown body without frontmatter, title, metadata callout, links, or empty sections. */
  body: string;
  /** Sections the projection has not filled yet ("_Not recorded._"). */
  pending: string[];
  /** Outgoing connections declared in the `## Links` section. */
  connections: Connection[];
  /** Every resolved outgoing target key (connections + inline wikilinks). */
  links: string[];
  outline: Heading[];
  excerpt: string;
  raw: string;
}

export interface Vault {
  docs: VaultDoc[];
  resolve: (target: string) => VaultDoc | undefined;
  backlinks: (key: string) => VaultDoc[];
}

const HIDDEN_FRONTMATTER = new Set([
  "id",
  "aliases",
  "type",
  "tags",
  "challenge_id",
  "challenge_slug",
  "coordinator_version",
]);
const WIKILINK =/\[\[([^\]|#]+)(?:#[^\]|]*)?(?:\|([^\]]+))?\]\]/g;
const EMPTY_SECTION = /^_(not recorded|see linked experiments and findings)\._?$/i;

const KIND_BY_TYPE: Record<string, DocKind> = {
  mission: "mission",
  "active-state": "active",
  dashboard: "dashboard",
  hypothesis: "hypothesis",
  experiment: "experiment",
  finding: "finding",
};

const GROUP_BY_KIND: Record<DocKind, DocGroup> = {
  mission: "project",
  active: "project",
  dashboard: "project",
  note: "project",
  hypothesis: "hypotheses",
  experiment: "experiments",
  finding: "findings",
};

const FIXED_TITLES: Partial<Record<DocKind, string>> = {
  mission: "Mission",
  active: "Active state",
  dashboard: "Dashboard",
};

export const GROUP_LABELS: Record<DocGroup, string> = {
  project: "Project",
  hypotheses: "Hypotheses",
  experiments: "Experiments",
  findings: "Findings",
};

export function headingId(text: string): string {
  return (
    text
      .toLowerCase()
      .replace(/[`*_~[\]]/g, "")
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "") || "section"
  );
}

function parseScalar(raw: string): string | string[] {
  const value = raw.trim();
  if (value.startsWith("[") || value.startsWith('"')) {
    try {
      const parsed = JSON.parse(value);
      if (Array.isArray(parsed)) return parsed.map(String);
      return String(parsed);
    } catch {
      /* fall through to the raw value */
    }
  }
  return value.replace(/^['"]|['"]$/g, "");
}

export function splitFrontmatter(source: string): { data: Record<string, string | string[]>; body: string } {
  const match = source.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?/);
  if (!match) return { data: {}, body: source };
  const data: Record<string, string | string[]> = {};
  for (const line of match[1].split(/\r?\n/)) {
    const pair = line.match(/^([A-Za-z0-9_-]+):\s*(.*)$/);
    if (pair) data[pair[1]] = parseScalar(pair[2]);
  }
  return { data, body: source.slice(match[0].length) };
}

/** Split a Markdown body into its preamble and `## ` sections, ignoring fenced code blocks. */
function sections(body: string): { preamble: string; items: { heading: string; content: string }[] } {
  const lines = body.split(/\r?\n/);
  const items: { heading: string; content: string[] }[] = [];
  const preamble: string[] = [];
  let fenced = false;
  for (const line of lines) {
    if (/^\s*(```|~~~)/.test(line)) fenced = !fenced;
    const heading = !fenced && line.match(/^##\s+(.+?)\s*#*\s*$/);
    if (heading) {
      items.push({ heading: heading[1], content: [] });
    } else if (items.length) {
      items[items.length - 1].content.push(line);
    } else {
      preamble.push(line);
    }
  }
  return {
    preamble: preamble.join("\n"),
    items: items.map((item) => ({ heading: item.heading, content: item.content.join("\n").trim() })),
  };
}

function wikiTargets(markdown: string): string[] {
  const targets: string[] = [];
  for (const match of markdown.matchAll(WIKILINK)) targets.push(match[1].trim());
  return targets;
}

function parseConnections(content: string): Connection[] {
  const connections: Connection[] = [];
  for (const line of content.split(/\r?\n/)) {
    const item = line.match(/^\s*[-*]\s*(?:([^:[]+):\s*)?(.*)$/);
    if (!item) continue;
    for (const target of wikiTargets(item[2])) {
      connections.push({ label: (item[1] ?? "").trim() || "Link", target });
    }
  }
  return connections;
}

function stripInline(markdown: string): string {
  return markdown
    .replace(WIKILINK, (_all, target: string, label?: string) => label ?? target)
    .replace(/[`*_>#]/g, "")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/\s+/g, " ")
    .trim();
}

function parseDoc(path: string, raw: string): VaultDoc {
  const { data, body } = splitFrontmatter(raw);
  const stem = path.split("/").pop()!.replace(/\.md$/i, "");
  const type = typeof data.type === "string" ? data.type : "";
  const kind: DocKind = KIND_BY_TYPE[type] ?? "note";
  const aliases = Array.isArray(data.aliases) ? data.aliases : data.aliases ? [String(data.aliases)] : [];
  const id = typeof data.id === "string" ? data.id : undefined;
  const key = id ?? aliases[0] ?? stem;

  const { preamble, items } = sections(body);
  const titleLine = preamble.match(/^#\s+(.+)$/m)?.[1]?.trim() ?? stem;
  let title = FIXED_TITLES[kind] ?? titleLine;
  const dash = titleLine.match(/^\S+\s+[—–-]\s+(.+)$/);
  if (!FIXED_TITLES[kind] && dash && id && titleLine.startsWith(id)) title = dash[1].trim();

  // The preamble after the title is the metadata callout (`> **Status**: …`) and any intro prose.
  const properties: Record<string, string> = {};
  const intro: string[] = [];
  for (const line of preamble.replace(/^#\s+.+$/m, "").split(/\r?\n/)) {
    const meta = line.match(/^>\s*\*\*([^*]+)\*\*:\s*(.*)$/);
    if (meta) {
      const value = meta[2].trim();
      if (value) properties[meta[1].trim()] = value;
    } else if (!/^>\s*$/.test(line)) {
      intro.push(line);
    }
  }
  for (const [name, value] of Object.entries(data)) {
    if (HIDDEN_FRONTMATTER.has(name) || Array.isArray(value) || !value) continue;
    const label = name.replace(/_/g, " ").replace(/^\w/, (letter) => letter.toUpperCase());
    if (!(label in properties) && !Object.keys(properties).some((key) => key.toLowerCase() === label.toLowerCase())) {
      properties[label] = value;
    }
  }

  let connections: Connection[] = [];
  const pending: string[] = [];
  const kept: string[] = [];
  const introText = intro.join("\n").trim();
  if (introText) kept.push(introText);
  for (const section of items) {
    if (section.heading.toLowerCase() === "links") {
      connections = parseConnections(section.content);
      continue;
    }
    if (!section.content || EMPTY_SECTION.test(section.content)) {
      pending.push(section.heading);
      continue;
    }
    if (section.content === "- [ ] No observations recorded.") {
      pending.push(section.heading);
      continue;
    }
    kept.push(`## ${section.heading}\n\n${section.content}`);
  }
  const cleanBody = kept.join("\n\n");

  const outline: Heading[] = [];
  let fenced = false;
  for (const line of cleanBody.split(/\r?\n/)) {
    if (/^\s*(```|~~~)/.test(line)) fenced = !fenced;
    const heading = !fenced && line.match(/^(##|###)\s+(.+?)\s*#*\s*$/);
    if (heading) {
      const text = stripInline(heading[2]);
      outline.push({ depth: heading[1].length as 2 | 3, text, id: headingId(text) });
    }
  }

  const firstParagraph =
    cleanBody
      .split(/\n\s*\n/)
      .map((block) => block.trim())
      .find((block) => block && !block.startsWith("#") && !block.startsWith("|")) ?? "";

  const status = typeof data.status === "string" ? data.status : properties.Status ?? null;

  return {
    key,
    path,
    kind,
    group: GROUP_BY_KIND[kind],
    title,
    aliases: Array.from(new Set([key, ...aliases, stem])),
    status,
    properties,
    body: cleanBody,
    pending,
    connections,
    links: [],
    outline,
    excerpt: stripInline(firstParagraph).slice(0, 280),
    raw,
  };
}

const KIND_ORDER: DocKind[] = ["mission", "active", "dashboard", "note", "hypothesis", "experiment", "finding"];

export function buildVault(files: Record<string, string>): Vault {
  const docs = Object.entries(files)
    .filter(([path]) => path.toLowerCase().endsWith(".md"))
    .map(([path, raw]) => parseDoc(path, raw))
    .sort((a, b) => {
      const order = KIND_ORDER.indexOf(a.kind) - KIND_ORDER.indexOf(b.kind);
      return order || a.key.localeCompare(b.key, undefined, { numeric: true });
    });

  const index = new Map<string, VaultDoc>();
  for (const doc of docs) {
    for (const alias of doc.aliases) {
      const normalized = alias.toLowerCase();
      if (!index.has(normalized)) index.set(normalized, doc);
    }
  }
  const resolve = (target: string) => index.get(target.trim().toLowerCase().replace(/\.md$/, ""));

  const incoming = new Map<string, Set<string>>();
  for (const doc of docs) {
    const targets = new Set<string>();
    for (const target of [...doc.connections.map((item) => item.target), ...wikiTargets(doc.body)]) {
      const resolved = resolve(target);
      if (resolved && resolved.key !== doc.key) targets.add(resolved.key);
    }
    doc.links = [...targets];
    for (const target of targets) {
      if (!incoming.has(target)) incoming.set(target, new Set());
      incoming.get(target)!.add(doc.key);
    }
  }
  const byKey = new Map(docs.map((doc) => [doc.key, doc]));
  const backlinks = (key: string) =>
    [...(incoming.get(key) ?? [])]
      .map((source) => byKey.get(source)!)
      .filter(Boolean)
      .sort((a, b) => KIND_ORDER.indexOf(a.kind) - KIND_ORDER.indexOf(b.kind) || a.key.localeCompare(b.key, undefined, { numeric: true }));

  return { docs, resolve, backlinks };
}

/** Rewrite Obsidian wikilinks into Markdown links the renderer understands (outside code). */
export function linkifyWikilinks(markdown: string): string {
  const parts = markdown.split(/(```[\s\S]*?```|`[^`\n]*`)/g);
  return parts
    .map((part, index) =>
      index % 2 === 1
        ? part
        : part.replace(WIKILINK, (_all, target: string, label?: string) => {
            const text = (label ?? target).trim().replace(/[[\]]/g, "");
            return `[${text}](wiki:${encodeURIComponent(target.trim())})`;
          }),
    )
    .join("");
}

/** Lightweight full-text match used by the quick switcher and knowledge filter. */
export function searchVault(vault: Vault, query: string, limit = 50): VaultDoc[] {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!terms.length) return vault.docs.slice(0, limit);
  const scored = vault.docs
    .map((doc) => {
      const head = `${doc.key} ${doc.title}`.toLowerCase();
      const body = doc.body.toLowerCase();
      let score = 0;
      for (const term of terms) {
        if (head.includes(term)) score += 10;
        else if (body.includes(term)) score += 1;
        else return { doc, score: 0 };
      }
      return { doc, score };
    })
    .filter((item) => item.score > 0)
    .sort((a, b) => b.score - a.score);
  return scored.slice(0, limit).map((item) => item.doc);
}
