import type { Artifact } from "./types";

export interface KnowledgeFilter {
  query?: string;
  kind?: string;
  status?: string;
  tag?: string;
  parent?: string;
  cursor?: string;
  limit?: number;
}

/** A labelled preview, never a replacement for the complete source. */
export function headline(source: string, limit = 140): { text: string; shortened: boolean } {
  const plain = source
    .replace(/\[\[([^\]|]+)(?:\|([^\]]+))?\]\]/g, (_match, id: string, label?: string) => label ?? id)
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/^[\s#>*+-]+/gm, "")
    .replace(/[`*_]/g, "")
    .replace(/\s+/g, " ")
    .trim();
  if (plain.length <= limit) return { text: plain, shortened: false };
  const sentence = plain.match(/^(.+?[.!?])(?:\s|$)/)?.[1];
  if (sentence && sentence.length <= limit) return { text: sentence, shortened: true };
  const cut = plain.slice(0, limit + 1);
  const boundary = cut.lastIndexOf(" ");
  return { text: `${cut.slice(0, boundary > limit / 2 ? boundary : limit).trimEnd()}…`, shortened: true };
}

export function artifactPreview(artifact: Artifact): string {
  for (const key of ["finding", "statement", "objective", "evidence", "analysis"]) {
    const value = artifact.content[key];
    if (typeof value === "string" && value.trim()) return headline(value, 200).text;
  }
  return "";
}

export const KNOWLEDGE_KINDS = [
  { value: "H", path: "hypotheses", label: "Hypotheses", description: "Claims to test" },
  { value: "E", path: "experiments", label: "Experiments", description: "Tests and observations" },
  { value: "F", path: "findings", label: "Findings", description: "Evidence and conclusions" },
];
