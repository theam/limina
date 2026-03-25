#!/usr/bin/env python3
"""
Claims completeness audit — LLM-based pre-process for the verification pipeline.

Generates per-draft audit prompts for the claims-auditor agent, which identifies
unsourced assertions and overclaims. Runs in parallel with Phase 1 (extraction).

Subcommands:
  generate  — Produce audit prompts (one per draft) as JSON
  apply     — Parse agent results and write references/audit-report.md

Stdlib-only. No external dependencies.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


# ---------------------------------------------------------------------------
# Prompt generation
# ---------------------------------------------------------------------------

def generate_audit_prompt(draft_path: Path) -> str:
    """Generate an audit prompt for one draft file."""
    return (
        f"# Claims Completeness Audit\n"
        f"\n"
        f"Read the full article at `{draft_path}` and perform a claims completeness audit.\n"
        f"\n"
        f"For every substantive assertion (statistics, factual claims, causal claims,\n"
        f"comparisons, predictions, specific numbers/dates), classify it as:\n"
        f"\n"
        f"- **CITED** — has an explicit citation → skip\n"
        f"- **SELF_SUPPORTED** — follows from the article's own argumentation → skip\n"
        f"- **COMMON_KNOWLEDGE** — widely accepted, no citation needed → skip\n"
        f"- **UNSOURCED** — specific factual claim with no citation → FLAG\n"
        f"- **OVERCLAIM** — goes beyond what cited evidence supports → FLAG\n"
        f"\n"
        f"Return ONLY the UNSOURCED and OVERCLAIM items as a JSON array.\n"
        f"See your agent definition for the full output format, severity levels,\n"
        f"and guidance on what to flag vs. skip.\n"
    )


def generate_all_prompts(drafts_dir: Path) -> list[dict]:
    """Generate audit prompts for all drafts. Returns list of {draft, prompt}."""
    draft_files = sorted(drafts_dir.glob("*.md"))
    if not draft_files:
        print(f"No markdown files found in {drafts_dir}", file=sys.stderr)
        return []

    prompts = []
    for draft_path in draft_files:
        prompts.append({
            "draft": draft_path.name,
            "prompt": generate_audit_prompt(draft_path),
        })

    return prompts


# ---------------------------------------------------------------------------
# Result application & report generation
# ---------------------------------------------------------------------------

def parse_results_file(path: Path) -> list[dict]:
    """Parse a JSON results file. Accepts either a single array or a
    wrapper object with a 'findings' key."""
    raw = json.loads(path.read_text(encoding="utf-8"))
    if isinstance(raw, list):
        return raw
    if isinstance(raw, dict) and "findings" in raw:
        return raw["findings"]
    return []


def load_all_results(results_dir: Path) -> dict[str, list[dict]]:
    """Load audit results grouped by draft.

    Expects files named like: audit-00-flagship.json or audit-01-embeddings.json
    Each file is a JSON array of findings for that draft.
    """
    results: dict[str, list[dict]] = {}
    for path in sorted(results_dir.glob("audit-*.json")):
        # Extract draft identifier from filename
        # e.g., "audit-00-flagship.json" → "00-flagship"
        stem = path.stem  # "audit-00-flagship"
        draft_slug = stem.replace("audit-", "", 1)
        findings = parse_results_file(path)
        results[draft_slug] = findings
    return results


def generate_report(results: dict[str, list[dict]], output_path: Path) -> None:
    """Generate the audit report markdown file."""
    total_overclaim = 0
    total_unsourced = 0
    total_high = 0
    total_medium = 0
    total_low = 0

    sections: list[str] = []

    for draft_slug in sorted(results.keys()):
        findings = results[draft_slug]
        overclaims = [f for f in findings if f.get("category") == "OVERCLAIM"]
        unsourced = [f for f in findings if f.get("category") == "UNSOURCED"]

        total_overclaim += len(overclaims)
        total_unsourced += len(unsourced)

        for f in findings:
            sev = f.get("severity", "MEDIUM").upper()
            if sev == "HIGH":
                total_high += 1
            elif sev == "LOW":
                total_low += 1
            else:
                total_medium += 1

        section_lines = [f"### {draft_slug}", ""]

        if not findings:
            section_lines.append("No issues found.\n")
            sections.append("\n".join(section_lines))
            continue

        if overclaims:
            section_lines.append("**OVERCLAIM**\n")
            for f in overclaims:
                sev = f.get("severity", "MEDIUM")
                line = f.get("line_number", "?")
                section_lines.append(
                    f"- [{sev}] **Line ~{line}**: "
                    f"\"{f.get('assertion', '?')}\" — {f.get('issue', '?')}"
                )
            section_lines.append("")

        if unsourced:
            section_lines.append("**UNSOURCED**\n")
            for f in unsourced:
                sev = f.get("severity", "MEDIUM")
                line = f.get("line_number", "?")
                section_lines.append(
                    f"- [{sev}] **Line ~{line}**: "
                    f"\"{f.get('assertion', '?')}\" — {f.get('issue', '?')}"
                )
            section_lines.append("")

        sections.append("\n".join(section_lines))

    total = total_overclaim + total_unsourced

    report = (
        f"# Claims Completeness Audit Report\n"
        f"\n"
        f"## Summary\n"
        f"\n"
        f"| Metric | Count |\n"
        f"|---|---|\n"
        f"| Drafts audited | {len(results)} |\n"
        f"| Total flags | {total} |\n"
        f"| OVERCLAIM | {total_overclaim} |\n"
        f"| UNSOURCED | {total_unsourced} |\n"
        f"| HIGH severity | {total_high} |\n"
        f"| MEDIUM severity | {total_medium} |\n"
        f"| LOW severity | {total_low} |\n"
        f"\n"
        f"## Findings by Draft\n"
        f"\n"
        + "\n".join(sections)
    )

    output_path.write_text(report, encoding="utf-8")
    print(f"Audit report written to {output_path}")
    print(f"  {total} flags: {total_overclaim} overclaim, {total_unsourced} unsourced")
    print(f"  Severity: {total_high} HIGH, {total_medium} MEDIUM, {total_low} LOW")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Claims completeness audit — LLM-based pre-process."
    )
    sub = parser.add_subparsers(dest="command", required=True)

    gen = sub.add_parser("generate", help="Generate audit prompts (one per draft)")
    gen.add_argument("drafts_dir", help="Directory containing markdown drafts")
    gen.add_argument(
        "--output", "-o", default="-",
        help="Output file for JSON prompts (default: stdout)",
    )

    apply_cmd = sub.add_parser("apply", help="Apply audit results and generate report")
    apply_cmd.add_argument(
        "results_dir",
        help="Directory containing audit-*.json result files",
    )
    apply_cmd.add_argument(
        "--output", "-o", default="references/audit-report.md",
        help="Output path for the audit report (default: references/audit-report.md)",
    )

    return parser.parse_args()


def main() -> int:
    args = parse_args()

    if args.command == "generate":
        drafts_dir = Path(args.drafts_dir).resolve()
        prompts = generate_all_prompts(drafts_dir)

        output = json.dumps(prompts, indent=2, ensure_ascii=False)
        if args.output == "-":
            print(output)
        else:
            Path(args.output).write_text(output, encoding="utf-8")
            print(
                f"Generated {len(prompts)} audit prompts",
                file=sys.stderr,
            )
        return 0

    elif args.command == "apply":
        results_dir = Path(args.results_dir).resolve()
        output_path = Path(args.output).resolve()
        output_path.parent.mkdir(parents=True, exist_ok=True)

        results = load_all_results(results_dir)
        if not results:
            print(f"No audit-*.json files found in {results_dir}", file=sys.stderr)
            return 1

        generate_report(results, output_path)
        return 0

    return 1


if __name__ == "__main__":
    raise SystemExit(main())
