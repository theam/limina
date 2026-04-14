#!/usr/bin/env python3
"""
Reconcile verification results against drafts and literature files.

Reads completed claims.csv, cross-references with literature files,
identifies propagation errors, and generates:
  - references/verification-report.md
  - references/fix-suggestions.md

Stdlib-only. No external dependencies.
"""

from __future__ import annotations

import argparse
import csv
import re
import sys
from collections import Counter
from dataclasses import dataclass
from pathlib import Path


# ---------------------------------------------------------------------------
# Data
# ---------------------------------------------------------------------------

@dataclass
class ClaimResult:
    claim_id: str
    source_id: str
    draft_file: str
    line_number: int
    claim_type: str
    claim_text: str
    extracted_assertion: str
    verification_status: str
    source_evidence: str
    discrepancy_detail: str
    is_propagation_error: bool = False
    lit_file_has_same_error: str = ""


def read_csv(path: Path) -> list[dict[str, str]]:
    with open(path, newline='', encoding='utf-8') as f:
        return list(csv.DictReader(f))


def load_claims(refs_dir: Path) -> list[ClaimResult]:
    rows = read_csv(refs_dir / "claims.csv")
    results = []
    for row in rows:
        results.append(ClaimResult(
            claim_id=row.get("claim_id", ""),
            source_id=row.get("source_id", ""),
            draft_file=row.get("draft_file", ""),
            line_number=int(row.get("line_number", 0)),
            claim_type=row.get("claim_type", ""),
            claim_text=row.get("claim_text", ""),
            extracted_assertion=row.get("extracted_assertion", ""),
            verification_status=row.get("verification_status", "PENDING"),
            source_evidence=row.get("source_evidence", ""),
            discrepancy_detail=row.get("discrepancy_detail", ""),
        ))
    return results


def load_sources(refs_dir: Path) -> dict[str, dict[str, str]]:
    rows = read_csv(refs_dir / "sources.csv")
    return {row["source_id"]: row for row in rows}


# ---------------------------------------------------------------------------
# Propagation error detection
# ---------------------------------------------------------------------------

def check_propagation_errors(claims: list[ClaimResult],
                             sources: dict[str, dict[str, str]],
                             project_root: Path) -> None:
    """Check if CONTRADICTED claims also appear in literature files."""
    lit_dir = project_root / "kb" / "research" / "literature"
    if not lit_dir.exists():
        return

    for claim in claims:
        if claim.verification_status != "CONTRADICTED":
            continue

        source = sources.get(claim.source_id, {})
        lit_file = source.get("lit_file", "")
        if not lit_file:
            continue

        lit_path = lit_dir / lit_file
        if not lit_path.exists():
            continue

        lit_text = lit_path.read_text(encoding="utf-8").lower()

        # Check if the wrong claim text appears in the literature file
        # Extract the key assertion (numbers, percentages, etc.)
        claim_lower = claim.claim_text.lower()

        # Look for specific numbers/percentages in the lit file
        numbers = re.findall(r'\d+(?:\.\d+)?%|\d+:\d+|\d+(?:\.\d+)?x', claim_lower)
        if numbers:
            for num in numbers:
                if num in lit_text:
                    claim.is_propagation_error = True
                    claim.lit_file_has_same_error = lit_file
                    break


# ---------------------------------------------------------------------------
# Report generation
# ---------------------------------------------------------------------------

def generate_report(claims: list[ClaimResult],
                    sources: dict[str, dict[str, str]]) -> str:
    """Generate verification-report.md content."""
    lines = ["# Reference Verification Report", ""]

    # Summary
    status_counts = Counter(c.verification_status for c in claims)
    total = len(claims)
    lines.extend([
        "## Summary",
        "",
        f"Total claims verified: **{total}**",
        "",
        "| Status | Count | % |",
        "|---|---|---|",
    ])
    for status in ["CONFIRMED", "CONTRADICTED", "NOT_FOUND", "PAYWALLED", "PENDING"]:
        count = status_counts.get(status, 0)
        pct = f"{count/total*100:.1f}" if total else "0"
        lines.append(f"| {status} | {count} | {pct}% |")
    lines.append("")

    # Propagation errors
    propagation = [c for c in claims if c.is_propagation_error]
    if propagation:
        lines.extend([
            "## Propagation Errors",
            "",
            "These errors exist in BOTH the draft AND the corresponding literature file,",
            "indicating the error propagated from research notes to the article.",
            "",
        ])
        for c in propagation:
            source = sources.get(c.source_id, {})
            lines.extend([
                f"### {c.claim_id} (Propagation: {c.lit_file_has_same_error})",
                f"- **Draft**: `{c.draft_file}` line {c.line_number}",
                f"- **Source**: {source.get('source_name', '?')} — \"{source.get('title', '?')}\"",
                f"- **Draft says**: {c.claim_text}",
                f"- **Source says**: {c.source_evidence}",
                f"- **Discrepancy**: {c.discrepancy_detail}",
                f"- **Also wrong in**: `kb/research/literature/{c.lit_file_has_same_error}`",
                "",
            ])

    # Critical issues (CONTRADICTED)
    contradicted = [c for c in claims if c.verification_status == "CONTRADICTED"
                    and not c.is_propagation_error]
    if contradicted:
        lines.extend([
            "## Contradicted Claims",
            "",
            "These claims are contradicted by the source material.",
            "",
        ])
        for c in contradicted:
            source = sources.get(c.source_id, {})
            lines.extend([
                f"### {c.claim_id}",
                f"- **Draft**: `{c.draft_file}` line {c.line_number}",
                f"- **Source**: {source.get('source_name', '?')} — \"{source.get('title', '?')}\"",
                f"- **Draft says**: {c.claim_text}",
                f"- **Source says**: {c.source_evidence}",
                f"- **Discrepancy**: {c.discrepancy_detail}",
                "",
            ])

    # Warnings (NOT_FOUND)
    not_found = [c for c in claims if c.verification_status == "NOT_FOUND"]
    if not_found:
        lines.extend([
            "## Not Found",
            "",
            "These claims could not be verified — the source was accessible but",
            "the specific claim was not found.",
            "",
        ])
        for c in not_found:
            source = sources.get(c.source_id, {})
            lines.extend([
                f"- **{c.claim_id}** (`{c.draft_file}` L{c.line_number}): "
                f"{c.claim_text[:100]} — Source: {source.get('source_name', '?')}",
            ])
        lines.append("")

    # Paywalled
    paywalled = [c for c in claims if c.verification_status == "PAYWALLED"]
    if paywalled:
        lines.extend([
            "## Paywalled (Manual Verification Needed)",
            "",
        ])
        paywalled_sources: dict[str, list[ClaimResult]] = {}
        for c in paywalled:
            paywalled_sources.setdefault(c.source_id, []).append(c)
        for sid, source_claims in paywalled_sources.items():
            source = sources.get(sid, {})
            lines.extend([
                f"### {source.get('source_name', '?')} — \"{source.get('title', '?')}\"",
                f"- URL: {source.get('url', 'N/A')}",
                f"- Claims: {len(source_claims)}",
            ])
            for c in source_claims:
                lines.append(f"  - {c.claim_id}: {c.claim_text[:100]}")
            lines.append("")

    # Confirmed (summary only)
    confirmed = [c for c in claims if c.verification_status == "CONFIRMED"]
    if confirmed:
        lines.extend([
            "## Confirmed Claims",
            "",
            f"{len(confirmed)} claims were confirmed by source material.",
            "",
        ])

    return "\n".join(lines)


def generate_fix_suggestions(claims: list[ClaimResult],
                             sources: dict[str, dict[str, str]]) -> str:
    """Generate fix-suggestions.md content."""
    fixable = [c for c in claims if c.verification_status == "CONTRADICTED"]
    if not fixable:
        return "# Fix Suggestions\n\nNo contradicted claims found. Nothing to fix.\n"

    lines = ["# Fix Suggestions", ""]
    lines.extend([
        "Each entry below shows what the draft says, what the source actually says,",
        "and a suggested replacement. Apply fixes to both drafts AND literature files",
        "where applicable.",
        "",
    ])

    for c in fixable:
        source = sources.get(c.source_id, {})
        lines.extend([
            f"## {c.claim_id}: `{c.draft_file}` line {c.line_number}",
            "",
            f"**Source**: {source.get('source_name', '?')} — \"{source.get('title', '?')}\"",
            "",
            f"**Draft text**: {c.claim_text}",
            "",
            f"**Source evidence**: {c.source_evidence}",
            "",
            f"**Discrepancy**: {c.discrepancy_detail}",
            "",
            f"**Suggested fix**: Replace the draft text with what the source actually says.",
            "",
        ])
        if c.is_propagation_error:
            lines.extend([
                f"**Also fix in**: `kb/research/literature/{c.lit_file_has_same_error}`",
                "",
            ])
        lines.append("---")
        lines.append("")

    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Reconcile verification results against drafts and literature."
    )
    parser.add_argument(
        "--refs-dir", default="references",
        help="Directory containing sources.csv and claims.csv",
    )
    parser.add_argument(
        "--project-root", default=".",
        help="Project root (for finding kb/research/literature/)",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    refs_dir = Path(args.refs_dir).resolve()
    project_root = Path(args.project_root).resolve()

    claims_path = refs_dir / "claims.csv"
    sources_path = refs_dir / "sources.csv"

    if not claims_path.exists():
        print(f"Error: {claims_path} not found", file=sys.stderr)
        return 1
    if not sources_path.exists():
        print(f"Error: {sources_path} not found", file=sys.stderr)
        return 1

    # Load data
    claims = load_claims(refs_dir)
    sources = load_sources(refs_dir)

    # Check for propagation errors
    check_propagation_errors(claims, sources, project_root)

    # Generate reports
    report = generate_report(claims, sources)
    report_path = refs_dir / "verification-report.md"
    report_path.write_text(report, encoding="utf-8")

    fixes = generate_fix_suggestions(claims, sources)
    fixes_path = refs_dir / "fix-suggestions.md"
    fixes_path.write_text(fixes, encoding="utf-8")

    # Summary
    status_counts = Counter(c.verification_status for c in claims)
    propagation = sum(1 for c in claims if c.is_propagation_error)

    print("Reconciliation complete.")
    print(f"  Total claims: {len(claims)}")
    for status, count in sorted(status_counts.items()):
        print(f"  {status}: {count}")
    if propagation:
        print(f"  Propagation errors: {propagation}")
    print(f"\nReports written:")
    print(f"  {report_path}")
    print(f"  {fixes_path}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
