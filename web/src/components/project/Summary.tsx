import { ChevronDown } from "@carbon/icons-react";
import { headline } from "../../lib/knowledge";
import { Markdown } from "../knowledge/Markdown";

/** Native disclosure keeps the complete source accessible by keyboard and touch. */
export function Summary({ label, source, slug, fallback = "Not recorded yet.", readLabel }: { label: string; source: string; slug: string; fallback?: string; readLabel?: string }) {
  if (!source.trim()) return <div className="summary-empty"><h2 className="label">{label}</h2><p>{fallback}</p></div>;
  return (
    <details className="summary-disclosure">
      <summary>
        <span className="label">{label}</span>
        <span className="summary-headline">{headline(source).text}</span>
        <span className="section-link summary-affordance"><span className="summary-read">{readLabel ?? `Read full ${label.toLowerCase()}`}</span><span className="summary-close">Hide details</span> <ChevronDown size={14} aria-hidden="true" /></span>
      </summary>
      <Markdown source={source} slug={slug} className="prose-compact summary-full" />
    </details>
  );
}
