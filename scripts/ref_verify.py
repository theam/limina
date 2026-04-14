#!/usr/bin/env python3
"""
Orchestrate reference verification by generating isolated prompts.

Reads sources.csv and claims.csv, groups claims by source, and generates
isolated verification prompts for the fact-checker agent. Each prompt
contains ONLY a URL and the claims to verify — no project context.

The actual agent spawning is done by the /verify-refs command.
This script handles CSV I/O and prompt generation.

Stdlib-only. No external dependencies.
"""

from __future__ import annotations

import argparse
import csv
import json
import sys
from pathlib import Path


# ---------------------------------------------------------------------------
# CSV I/O
# ---------------------------------------------------------------------------

def read_csv(path: Path) -> list[dict[str, str]]:
    with open(path, newline='', encoding='utf-8') as f:
        return list(csv.DictReader(f))


def write_csv(path: Path, rows: list[dict[str, str]], fieldnames: list[str]) -> None:
    with open(path, 'w', newline='', encoding='utf-8') as f:
        writer = csv.DictWriter(f, fieldnames=fieldnames)
        writer.writeheader()
        writer.writerows(rows)


# ---------------------------------------------------------------------------
# Prompt generation
# ---------------------------------------------------------------------------

URL_TYPE_HINTS = {
    "arxiv": (
        "This is an arXiv paper. Try fetching the HTML version first: "
        "replace /abs/ with /html/ in the URL. If that fails, try the "
        "abstract page at the original URL."
    ),
    "acl": (
        "This is an ACL Anthology paper. The page should contain the "
        "abstract and links to the PDF."
    ),
    "openreview": (
        "This is an OpenReview submission. The forum page contains the "
        "abstract and reviews."
    ),
    "usenix": (
        "This is a USENIX paper. The presentation page may have an "
        "abstract. If not, search for the paper title."
    ),
    "acm": (
        "This is an ACM Digital Library paper. It may be paywalled. "
        "If you can only see the abstract, report as PAYWALLED for "
        "claims not verifiable from the abstract."
    ),
    "springer": (
        "This is a Springer paper. It may be paywalled. If you can "
        "only see the abstract, report as PAYWALLED."
    ),
    "vldb": (
        "This is a VLDB paper. Try fetching the PDF URL directly."
    ),
    "x.com": (
        "This is a tweet/post on X (formerly Twitter). Use WebSearch "
        "to find the content, as direct fetch may not work. Search for "
        "the author name and key phrases from the claims."
    ),
    "pdf": (
        "This URL points to a PDF. Try fetching it. If the content is "
        "not readable, use WebSearch to find the paper by title."
    ),
    "web": (
        "This is a web page (blog post, company page, etc.). Fetch it "
        "directly."
    ),
    "none": (
        "No URL is available for this source. Use WebSearch to find the "
        "source by title and author name."
    ),
}


def generate_prompt(source: dict[str, str],
                    claims: list[dict[str, str]]) -> str:
    """Generate an isolated verification prompt for the fact-checker agent."""
    url_type = source.get("url_type", "web")
    url_hint = URL_TYPE_HINTS.get(url_type, URL_TYPE_HINTS["web"])

    url = source.get("url", "")
    title = source.get("title", "")
    source_name = source.get("source_name", "")
    year = source.get("year", "")

    lines = [
        "# Fact-Check Assignment",
        "",
        f"**Source**: {source_name}. \"{title}\" ({year})",
        f"**URL**: {url}" if url else "**URL**: Not available — use WebSearch",
        f"**URL type**: {url_type}",
        "",
        f"**Fetch hint**: {url_hint}",
        "",
        "## Claims to Verify",
        "",
    ]

    for i, claim in enumerate(claims, 1):
        lines.extend([
            f"### Claim {i} ({claim['claim_id']})",
            f"- **Type**: {claim['claim_type']}",
            f"- **Assertion**: {claim['extracted_assertion']}",
            f"- **Draft text**: {claim['claim_text']}",
            "",
        ])

    lines.extend([
        "## Instructions",
        "",
        "For EACH claim above, report ONE of:",
        "- **CONFIRMED**: The source contains information supporting this claim. "
        "Provide the exact quote or data from the source.",
        "- **CONTRADICTED**: The source says something different. Provide what "
        "the source actually says.",
        "- **NOT_FOUND**: You accessed the source but could not find information "
        "about this specific claim.",
        "- **PAYWALLED**: The source is behind a paywall and you cannot access "
        "the full content to verify.",
        "",
        "## Accuracy Rules",
        "",
        "- For **number** claims: exact match required. 92% ≠ 91%. 50:1 ≠ 45:1.",
        "- For **quote** claims: verbatim match required. Paraphrases are CONTRADICTED.",
        "- For **attribution** claims: verify both who said it AND what they said.",
        "- Always provide the exact text from the source as evidence.",
        "",
        "## Response Format",
        "",
        "Respond with a JSON array, one object per claim:",
        "```json",
        "[",
        '  {',
        '    "claim_id": "C001",',
        '    "status": "CONFIRMED|CONTRADICTED|NOT_FOUND|PAYWALLED",',
        '    "source_evidence": "Exact quote or data from source",',
        '    "discrepancy_detail": "Only if CONTRADICTED: what the source actually says"',
        '  }',
        "]",
        "```",
    ])

    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Batch management
# ---------------------------------------------------------------------------

def group_claims_by_source(claims: list[dict[str, str]]) -> dict[str, list[dict[str, str]]]:
    """Group claims by source_id, only including PENDING ones."""
    groups: dict[str, list[dict[str, str]]] = {}
    for claim in claims:
        if claim.get("verification_status", "PENDING") != "PENDING":
            continue
        sid = claim.get("source_id", "")
        if not sid:
            continue
        groups.setdefault(sid, []).append(claim)
    return groups


def generate_all_prompts(refs_dir: Path) -> list[dict]:
    """Generate all verification prompts. Returns list of {source_id, prompt, claim_ids}."""
    sources_path = refs_dir / "sources.csv"
    claims_path = refs_dir / "claims.csv"

    sources = {row["source_id"]: row for row in read_csv(sources_path)}
    claims = read_csv(claims_path)

    groups = group_claims_by_source(claims)
    prompts = []

    for source_id, source_claims in sorted(groups.items()):
        source = sources.get(source_id)
        if not source:
            print(f"Warning: source {source_id} not found in sources.csv", file=sys.stderr)
            continue

        # Batch into groups of max 5 claims
        for batch_start in range(0, len(source_claims), 5):
            batch = source_claims[batch_start:batch_start + 5]
            prompt = generate_prompt(source, batch)
            prompts.append({
                "source_id": source_id,
                "source_name": source.get("source_name", ""),
                "title": source.get("title", ""),
                "url": source.get("url", ""),
                "claim_ids": [c["claim_id"] for c in batch],
                "prompt": prompt,
            })

    return prompts


def update_claims_from_results(refs_dir: Path, results: list[dict]) -> None:
    """Update claims.csv with verification results.

    results: list of {claim_id, status, source_evidence, discrepancy_detail}
    """
    claims_path = refs_dir / "claims.csv"
    claims = read_csv(claims_path)

    result_map = {r["claim_id"]: r for r in results}

    for claim in claims:
        cid = claim["claim_id"]
        if cid in result_map:
            r = result_map[cid]
            claim["verification_status"] = r.get("status", "PENDING")
            claim["source_evidence"] = r.get("source_evidence", "")
            claim["discrepancy_detail"] = r.get("discrepancy_detail", "")

    fieldnames = list(claims[0].keys()) if claims else []
    write_csv(claims_path, claims, fieldnames)


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Generate or process reference verification prompts."
    )
    sub = parser.add_subparsers(dest="command", required=True)

    gen = sub.add_parser("generate", help="Generate verification prompts")
    gen.add_argument(
        "--refs-dir", default="references",
        help="Directory containing sources.csv and claims.csv",
    )
    gen.add_argument(
        "--output", "-o", default="-",
        help="Output file for prompts JSON (default: stdout)",
    )

    upd = sub.add_parser("update", help="Update claims with verification results")
    upd.add_argument(
        "--refs-dir", default="references",
        help="Directory containing claims.csv",
    )
    upd.add_argument(
        "results_file",
        help="JSON file with verification results",
    )

    return parser.parse_args()


def main() -> int:
    args = parse_args()

    if args.command == "generate":
        refs_dir = Path(args.refs_dir).resolve()
        prompts = generate_all_prompts(refs_dir)

        output = json.dumps(prompts, indent=2, ensure_ascii=False)
        if args.output == "-":
            print(output)
        else:
            Path(args.output).write_text(output, encoding="utf-8")

        # Print summary to stderr
        total_claims = sum(len(p["claim_ids"]) for p in prompts)
        print(f"Generated {len(prompts)} verification batches "
              f"covering {total_claims} claims", file=sys.stderr)
        return 0

    elif args.command == "update":
        refs_dir = Path(args.refs_dir).resolve()
        results_file = Path(args.results_file).resolve()
        results = json.loads(results_file.read_text(encoding="utf-8"))
        update_claims_from_results(refs_dir, results)
        print(f"Updated {len(results)} claims in {refs_dir / 'claims.csv'}")
        return 0

    return 1


if __name__ == "__main__":
    raise SystemExit(main())
