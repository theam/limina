import { readFile, readdir } from "fs/promises";
import { join, basename, relative } from "path";

// Artifact types matching the Limina research framework
export type ArtifactType =
  | "hypothesis"
  | "experiment"
  | "finding"
  | "challenge_review"
  | "strategic_review"
  | "report"
  | "task"
  | "literature";

export type ArtifactStatus = string;

export interface Artifact {
  type: ArtifactType;
  id: string;
  title: string;
  status: ArtifactStatus;
  metadata: Record<string, string>;
  content: string;
  filePath: string;
  lastModified?: Date;
}

export interface TaskEntry {
  id: string;
  title: string;
  status: string;
  priority: string;
  type: string;
}

export interface CeoRequest {
  id: string;
  title: string;
  status: "PENDING" | "RESOLVED" | "AUTO_RESOLVED";
  description: string;
  response?: string;
  date?: string;
}

export interface Directive {
  id: string;
  title: string;
  instruction: string;
  priority: "HIGH" | "NORMAL" | "LOW";
  status: "PENDING" | "ACKNOWLEDGED";
  submittedAt: string;
}

export interface KBState {
  artifacts: Artifact[];
  tasks: TaskEntry[];
  ceoRequests: CeoRequest[];
  directives: Directive[];
  phase: ResearchPhase;
  artifactCounts: {
    hypotheses: number;
    experiments: number;
    findings: number;
    challengeReviews: number;
    tasks: number;
  };
}

export type ResearchPhase =
  | "idle"
  | "hypothesizing"
  | "experimenting"
  | "documenting"
  | "reviewing";

// File path pattern → artifact type mapping
// Matches the Limina framework's kb/ structure
const PATH_PATTERNS: Array<{ pattern: RegExp; type: ArtifactType }> = [
  { pattern: /research\/hypotheses\/H\d{3}/, type: "hypothesis" },
  { pattern: /research\/experiments\/E\d{3}/, type: "experiment" },
  { pattern: /research\/findings\/F\d{3}/, type: "finding" },
  { pattern: /research\/literature\/L\d{3}/, type: "literature" },
  { pattern: /reports\/CR\d{3}/, type: "challenge_review" },
  { pattern: /reports\/SR\d{3}/, type: "strategic_review" },
  { pattern: /tasks\/T\d{3}/, type: "task" },
];

const ID_RE = /^(?:FT|INV|IMP|RET|CR|SR|T|H|E|F|L)\d{3}/;

/**
 * Extract metadata from the `> **Key**: Value` blockquote format
 * used by the Limina framework's artifact templates.
 */
export function parseMetadata(content: string): Record<string, string> {
  const metadata: Record<string, string> = {};
  const lines = content.split("\n");

  for (const line of lines) {
    const match = line.match(/^>\s+\*\*(.+?)\*\*:\s*(.+?)\s*$/);
    if (match) {
      metadata[match[1].toLowerCase()] = match[2];
    }
  }

  return metadata;
}

/**
 * Extract the title from an artifact markdown file.
 * Title is the first H1 heading: `# T001 — Title Here`
 */
export function parseTitle(content: string): string {
  const match = content.match(/^#\s+(.+)$/m);
  if (!match) return "Untitled";

  // Strip the artifact ID prefix if present
  const title = match[1].replace(/^(?:FT|INV|IMP|RET|CR|SR|T|H|E|F|L)\d{3}\s*[—–-]\s*/, "");
  return title.trim() || "Untitled";
}

/**
 * Extract the artifact ID from a filename.
 * e.g., "H001-negation-handling.md" → "H001"
 */
export function parseIdFromFilename(filename: string): string {
  const match = basename(filename).match(ID_RE);
  return match ? match[0] : "";
}

/**
 * Determine artifact type from file path.
 */
export function detectArtifactType(filePath: string): ArtifactType | null {
  for (const { pattern, type } of PATH_PATTERNS) {
    if (pattern.test(filePath)) {
      return type;
    }
  }
  return null;
}

/**
 * Parse a single artifact markdown file into a typed object.
 */
export function parseArtifact(
  content: string,
  filePath: string
): Artifact | null {
  let type = detectArtifactType(filePath);

  // Generic report: any .md in reports/ that isn't CR/SR
  if (!type && filePath.includes("reports/") && !filePath.includes("archive/")) {
    type = "report";
  }

  if (!type) return null;

  const metadata = parseMetadata(content);
  const title = parseTitle(content);
  let id = parseIdFromFilename(filePath);

  // For generic reports without standard IDs, use the filename as ID
  if (!id && type === "report") {
    const basename = filePath.split("/").pop()?.replace(".md", "") || "";
    id = basename;
  }

  return {
    type,
    id,
    title,
    status: metadata.status || "unknown",
    metadata,
    content,
    filePath,
  };
}

/**
 * Parse BACKLOG.md to extract task entries.
 * The backlog uses a markdown table format:
 * | ID | Title | Status | Priority | Type |
 */
export function parseBacklog(content: string): TaskEntry[] {
  const tasks: TaskEntry[] = [];
  const lines = content.split("\n");

  for (const line of lines) {
    // Match table rows: | T001 | Title | STATUS | P0 | research |
    const match = line.match(
      /\|\s*(T\d{3})\s*\|\s*(.+?)\s*\|\s*(\w+)\s*\|\s*(P\d)\s*\|\s*(\w+)\s*\|/
    );
    if (match) {
      tasks.push({
        id: match[1],
        title: match[2].trim(),
        status: match[3],
        priority: match[4],
        type: match[5],
      });
    }
  }

  return tasks;
}

/**
 * Parse CEO_REQUESTS.md to extract pending/resolved requests.
 * Format:
 * ## REQ-001: Title
 * > **Status**: PENDING
 * > **Date**: 2026-03-23
 * Description text...
 */
export function parseCeoRequests(content: string): CeoRequest[] {
  const requests: CeoRequest[] = [];
  const sections = content.split(/^## /m).filter(Boolean);

  for (const section of sections) {
    // Support multiple formats the agent may use:
    // "REQ-001: Title" or "CR-001 — Title" or "REQ-001 — Title" or any ID format
    const headerMatch = section.match(/^([A-Z]+-\d+)\s*[:\u2014\u2013-]\s*(.+)$/m);
    if (!headerMatch) continue;

    const metadata = parseMetadata(section);
    const status = (metadata.status || metadata.priority ? "PENDING" : "PENDING") as CeoRequest["status"];
    const parsedStatus = metadata.status as CeoRequest["status"] || "PENDING";

    // Extract description — everything that's not a header, metadata line, or sub-heading
    const lines = section.split("\n");
    const descLines = lines.filter(
      (l) =>
        !l.startsWith(">") &&
        !l.match(/^[A-Z]+-\d+/) &&
        !l.match(/^###/) &&
        !l.match(/^```/) &&
        l.trim()
    );

    requests.push({
      id: headerMatch[1],
      title: headerMatch[2].trim(),
      status: parsedStatus,
      description: descLines.join("\n").trim(),
      response: metadata.response,
      date: metadata.date,
    });
  }

  return requests;
}

/**
 * Parse DIRECTIVES.md to extract CEO directives.
 * Format:
 * ## DIR-001: Title
 * > **Priority**: HIGH
 * > **Submitted**: 2026-03-24T10:30:00Z
 * > **Status**: PENDING
 * Instruction text...
 */
export function parseDirectives(content: string): Directive[] {
  const directives: Directive[] = [];
  const sections = content.split(/^## /m).filter(Boolean);

  for (const section of sections) {
    const headerMatch = section.match(/^(DIR-\d+)\s*[:\u2014\u2013-]\s*(.+)$/m);
    if (!headerMatch) continue;

    const metadata = parseMetadata(section);

    const lines = section.split("\n");
    const instrLines = lines.filter(
      (l) =>
        !l.startsWith(">") &&
        !l.match(/^DIR-\d+/) &&
        !l.match(/^###/) &&
        l.trim()
    );

    directives.push({
      id: headerMatch[1],
      title: headerMatch[2].trim(),
      instruction: instrLines.join("\n").trim(),
      priority: (metadata.priority as Directive["priority"]) || "NORMAL",
      status: (metadata.status as Directive["status"]) || "PENDING",
      submittedAt: metadata.submitted || "",
    });
  }

  return directives;
}

/**
 * Infer the current research phase from the most recently created artifact.
 *
 * The research cycle is: H → E → F → CR/SR. Within a cycle, if both
 * H001 and E001 exist, E001 was created AFTER H001. We use type priority
 * to break ties when numeric IDs are equal.
 */
export function inferPhase(artifacts: Artifact[]): ResearchPhase {
  if (artifacts.length === 0) return "idle";

  // Type priority in the research cycle (higher = later in cycle)
  const typePriority: Record<string, number> = {
    task: 0,
    literature: 1,
    hypothesis: 2,
    experiment: 3,
    finding: 4,
    challenge_review: 5,
    strategic_review: 5,
  };

  // Sort by: numeric ID descending, then type priority descending
  const sorted = [...artifacts].sort((a, b) => {
    const numA = parseInt(a.id.replace(/\D/g, ""), 10) || 0;
    const numB = parseInt(b.id.replace(/\D/g, ""), 10) || 0;
    if (numB !== numA) return numB - numA;
    return (typePriority[b.type] ?? 0) - (typePriority[a.type] ?? 0);
  });

  const latest = sorted[0];

  switch (latest.type) {
    case "hypothesis":
      return "hypothesizing";
    case "experiment":
      return "experimenting";
    case "finding":
      return "documenting";
    case "challenge_review":
    case "strategic_review":
      return "reviewing";
    default:
      return "idle";
  }
}

/**
 * Read the full KB state from a mission's kb/ directory.
 * This is the main function called by the status polling endpoint.
 */
export async function readKBState(kbPath: string): Promise<KBState> {
  const artifacts: Artifact[] = [];

  const artifactDirs = [
    { dir: "research/hypotheses", type: "hypothesis" as const },
    { dir: "research/experiments", type: "experiment" as const },
    { dir: "research/findings", type: "finding" as const },
    { dir: "research/literature", type: "literature" as const },
    { dir: "reports", type: null }, // CR and SR both live here
    { dir: "tasks", type: "task" as const },
  ];

  for (const { dir } of artifactDirs) {
    const dirPath = join(kbPath, dir);
    try {
      const files = await readdir(dirPath);
      for (const file of files) {
        if (!file.endsWith(".md")) continue;
        const filePath = join(dirPath, file);
        try {
          const content = await readFile(filePath, "utf-8");
          const artifact = parseArtifact(content, relative(kbPath, filePath));
          if (artifact) {
            artifacts.push(artifact);
          }
        } catch {
          // Skip malformed files — don't crash the UI
        }
      }
    } catch {
      // Directory doesn't exist yet — normal during early mission phases
    }
  }

  // Parse tasks from BACKLOG.md
  let tasks: TaskEntry[] = [];
  try {
    const backlog = await readFile(join(kbPath, "mission/BACKLOG.md"), "utf-8");
    tasks = parseBacklog(backlog);
  } catch {
    // BACKLOG.md doesn't exist yet
  }

  // Parse CEO requests
  let ceoRequests: CeoRequest[] = [];
  try {
    const reqFile = await readFile(
      join(kbPath, "mission/CEO_REQUESTS.md"),
      "utf-8"
    );
    ceoRequests = parseCeoRequests(reqFile);
  } catch {
    // CEO_REQUESTS.md doesn't exist yet
  }

  // Parse directives
  let directives: Directive[] = [];
  try {
    const dirFile = await readFile(
      join(kbPath, "mission/DIRECTIVES.md"),
      "utf-8"
    );
    directives = parseDirectives(dirFile);
  } catch {
    // DIRECTIVES.md doesn't exist yet
  }

  const phase = inferPhase(artifacts);

  return {
    artifacts,
    tasks,
    ceoRequests,
    directives,
    phase,
    artifactCounts: {
      hypotheses: artifacts.filter((a) => a.type === "hypothesis").length,
      experiments: artifacts.filter((a) => a.type === "experiment").length,
      findings: artifacts.filter((a) => a.type === "finding").length,
      challengeReviews: artifacts.filter((a) => a.type === "challenge_review")
        .length,
      tasks: tasks.length,
    },
  };
}
