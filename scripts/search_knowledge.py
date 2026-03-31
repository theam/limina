#!/usr/bin/env python3
"""
Search Knowledge Cards by relevance to a query.

Usage:
    python3 scripts/search_knowledge.py "embedding models Spanish"
    python3 scripts/search_knowledge.py "latency optimization" --top 5
    python3 scripts/search_knowledge.py "RAG retrieval" --dir ~/other-project/shared-knowledge
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


META_RE = re.compile(r"^>\s+\*\*(.+?)\*\*:\s*(.+?)\s*$")
HEADING_RE = re.compile(r"^##?\s+(.+)$")


def parse_metadata(text: str) -> dict[str, str]:
    meta: dict[str, str] = {}
    for line in text.splitlines():
        m = META_RE.match(line.strip())
        if m:
            meta[m.group(1).strip()] = m.group(2).strip()
    return meta


def extract_key_finding(text: str) -> str:
    """Extract the first non-empty line after '## Key Finding'."""
    in_section = False
    for line in text.splitlines():
        if line.strip().startswith("## Key Finding"):
            in_section = True
            continue
        if in_section:
            stripped = line.strip()
            if stripped and not stripped.startswith("_") and not stripped.startswith("#"):
                return stripped
            if stripped.startswith("##"):
                break
    return ""


def score_card(query_terms: list[str], text_lower: str) -> float:
    """Score a card by term frequency + section weighting."""
    score = 0.0
    for term in query_terms:
        # Count raw occurrences
        count = text_lower.count(term)
        if count == 0:
            continue
        # Base score: log-ish scaling to avoid one repeated term dominating
        score += min(count, 5)
        # Bonus: term in title (first line)
        first_line = text_lower.split("\n", 1)[0]
        if term in first_line:
            score += 3
        # Bonus: term in Key Finding section
        kf_start = text_lower.find("## key finding")
        kf_end = text_lower.find("##", kf_start + 1) if kf_start != -1 else -1
        if kf_start != -1:
            kf_section = text_lower[kf_start:kf_end] if kf_end != -1 else text_lower[kf_start:]
            if term in kf_section:
                score += 2
    return score


def search(shared_dir: Path, query: str, top_n: int) -> list[dict]:
    cards_dir = shared_dir / "cards"
    if not cards_dir.exists():
        return []

    query_terms = [t.lower() for t in query.split() if len(t) > 2]
    if not query_terms:
        print("Query too short — use at least one term with 3+ characters.", file=sys.stderr)
        sys.exit(1)

    results = []
    for path in sorted(cards_dir.glob("K*.md")):
        text = path.read_text(encoding="utf-8")
        text_lower = text.lower()

        score = score_card(query_terms, text_lower)
        if score == 0:
            continue

        meta = parse_metadata(text)
        key_finding = extract_key_finding(text)

        results.append({
            "path": path,
            "id": path.stem.split("-", 1)[0],
            "title": path.stem.split("-", 1)[1].replace("-", " ") if "-" in path.stem else path.stem,
            "score": score,
            "domain": meta.get("Domain", "—"),
            "confidence": meta.get("Confidence", "—"),
            "source": meta.get("Source mission", "—"),
            "finding": key_finding,
        })

    results.sort(key=lambda r: r["score"], reverse=True)
    # Filter out low-relevance noise — require at least 2 term matches or a title/finding hit
    results = [r for r in results if r["score"] >= 4]
    return results[:top_n]


def main() -> int:
    parser = argparse.ArgumentParser(description="Search Knowledge Cards by relevance.")
    parser.add_argument("query", help="Search terms (space-separated)")
    parser.add_argument("--top", type=int, default=5, help="Max results (default: 5)")
    parser.add_argument(
        "--dir",
        default=None,
        help="Path to shared-knowledge/ directory (default: auto-detect)",
    )
    args = parser.parse_args()

    if args.dir:
        shared_dir = Path(args.dir).resolve()
    else:
        # Auto-detect: look for shared-knowledge/ in cwd or parent dirs
        cwd = Path.cwd()
        shared_dir = None
        for parent in [cwd] + list(cwd.parents):
            candidate = parent / "shared-knowledge"
            if candidate.exists():
                shared_dir = candidate
                break
        if shared_dir is None:
            print("No shared-knowledge/ directory found. Use --dir to specify.", file=sys.stderr)
            return 1

    results = search(shared_dir, args.query, args.top)

    if not results:
        print(f"No cards match: {args.query}")
        return 0

    print(f"Found {len(results)} relevant card(s) for: {args.query}\n")
    for r in results:
        print(f"  {r['id']}  [{r['confidence']}]  {r['title']}")
        print(f"         Domain: {r['domain']}  |  Source: {r['source']}  |  Score: {r['score']:.0f}")
        if r["finding"]:
            finding = r["finding"][:120] + "..." if len(r["finding"]) > 120 else r["finding"]
            print(f"         {finding}")
        print(f"         → {r['path']}")
        print()

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
