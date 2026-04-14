#!/usr/bin/env python3
"""
Triage extracted claims using LLM-based classification.

Reads claims.csv and sources.csv, groups claims into batches,
and generates triage prompts for the claim-reviewer agent.
After triage, updates claims.csv with classifications and filters
out EDITORIAL claims before verification.

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
# Triage prompt generation
# ---------------------------------------------------------------------------

def generate_triage_prompt(claims_batch: list[dict[str, str]],
                           sources: dict[str, dict[str, str]],
                           drafts_dir: Path) -> str:
    """Generate a triage prompt for the claim-reviewer agent."""
    # Collect unique draft files referenced
    draft_files = sorted({c['draft_file'] for c in claims_batch})

    lines = [
        "# Claim Triage Batch",
        "",
        "Review these extracted claims and classify each as VERIFY, VERIFY_REATTRIBUTE, EDITORIAL, or SYNTHESIS.",
        "",
        f"**Draft files to read for context**: {', '.join(draft_files)}",
        f"**Drafts directory**: {drafts_dir}",
        "",
        "## Claims",
        "",
    ]

    for i, claim in enumerate(claims_batch, 1):
        source = sources.get(claim.get('source_id', ''), {})
        source_name = source.get('source_name', 'Unknown')
        source_title = source.get('title', 'Unknown')

        lines.extend([
            f"### {claim['claim_id']} (line {claim['line_number']} of `{claim['draft_file']}`)",
            f"- **Type**: {claim['claim_type']}",
            f"- **Attributed to**: {claim.get('source_id', 'none')} — {source_name}: \"{source_title}\"",
            f"- **Claim text**: {claim['claim_text'][:200]}",
            f"- **Assertion**: {claim['extracted_assertion'][:200]}",
            "",
        ])

    lines.extend([
        "## Instructions",
        "",
        "1. Read the relevant draft files to see each claim in its full paragraph context.",
        "2. For each claim, determine: is this a real sourced claim (VERIFY), a misattributed sourced claim (VERIFY_REATTRIBUTE), our own editorial text (EDITORIAL), or our original synthesis (SYNTHESIS)?",
        "3. Return a JSON array with one object per claim.",
        "",
        "See your agent definition for detailed classification rules and output format.",
    ])

    return "\n".join(lines)


def generate_all_triage_prompts(refs_dir: Path,
                                 drafts_dir: Path,
                                 batch_size: int = 20) -> list[dict]:
    """Generate all triage prompts. Returns list of {prompt, claim_ids}."""
    sources_path = refs_dir / "sources.csv"
    claims_path = refs_dir / "claims.csv"

    sources = {row["source_id"]: row for row in read_csv(sources_path)}
    claims = [row for row in read_csv(claims_path)
              if row.get("verification_status", "PENDING") == "PENDING"]

    prompts = []
    for batch_start in range(0, len(claims), batch_size):
        batch = claims[batch_start:batch_start + batch_size]
        prompt = generate_triage_prompt(batch, sources, drafts_dir)
        prompts.append({
            "claim_ids": [c["claim_id"] for c in batch],
            "prompt": prompt,
        })

    return prompts


# ---------------------------------------------------------------------------
# Apply triage results
# ---------------------------------------------------------------------------

def apply_triage(refs_dir: Path, triage_results: list[dict]) -> dict[str, int]:
    """Apply triage classifications to claims.csv.

    triage_results: list of {claim_id, classification, correct_source_id, reason}

    Returns counts by classification.
    """
    claims_path = refs_dir / "claims.csv"
    claims = read_csv(claims_path)

    result_map = {r["claim_id"]: r for r in triage_results}
    counts: dict[str, int] = {}

    for claim in claims:
        cid = claim["claim_id"]
        if cid not in result_map:
            continue

        r = result_map[cid]
        classification = r.get("classification", "VERIFY")
        counts[classification] = counts.get(classification, 0) + 1

        if classification == "EDITORIAL":
            claim["verification_status"] = "NOT_FOUND"
            claim["source_evidence"] = f"Triage: EDITORIAL — {r.get('reason', '')}"
            claim["discrepancy_detail"] = "Filtered by claim-reviewer: not a sourced claim."
        elif classification == "SYNTHESIS":
            claim["verification_status"] = "NOT_FOUND"
            claim["source_evidence"] = f"Triage: SYNTHESIS — {r.get('reason', '')}"
            claim["discrepancy_detail"] = "Filtered by claim-reviewer: original synthesis, not verifiable against single source."
        elif classification == "VERIFY_REATTRIBUTE":
            new_source = r.get("correct_source_id", "")
            if new_source:
                claim["source_id"] = new_source
            # Keep as PENDING for verification with corrected source
        # VERIFY: keep as-is, PENDING for verification

    fieldnames = list(claims[0].keys()) if claims else []
    write_csv(claims_path, claims, fieldnames)

    return counts


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Triage extracted claims with LLM classification."
    )
    sub = parser.add_subparsers(dest="command", required=True)

    gen = sub.add_parser("generate", help="Generate triage prompts")
    gen.add_argument("--refs-dir", default="references")
    gen.add_argument("--drafts-dir", default="drafts")
    gen.add_argument("--batch-size", type=int, default=20)
    gen.add_argument("--output", "-o", default="-")

    apply = sub.add_parser("apply", help="Apply triage results")
    apply.add_argument("--refs-dir", default="references")
    apply.add_argument("results_file", help="JSON file with triage results")

    return parser.parse_args()


def main() -> int:
    args = parse_args()

    if args.command == "generate":
        refs_dir = Path(args.refs_dir).resolve()
        drafts_dir = Path(args.drafts_dir).resolve()
        prompts = generate_all_triage_prompts(refs_dir, drafts_dir, args.batch_size)

        output = json.dumps(prompts, indent=2, ensure_ascii=False)
        if args.output == "-":
            print(output)
        else:
            Path(args.output).write_text(output, encoding="utf-8")

        total_claims = sum(len(p["claim_ids"]) for p in prompts)
        print(f"Generated {len(prompts)} triage batches "
              f"covering {total_claims} claims", file=sys.stderr)
        return 0

    elif args.command == "apply":
        refs_dir = Path(args.refs_dir).resolve()
        results = json.loads(Path(args.results_file).read_text(encoding="utf-8"))
        counts = apply_triage(refs_dir, results)
        print("Triage applied:")
        for cls, count in sorted(counts.items()):
            print(f"  {cls}: {count}")
        return 0

    return 1


if __name__ == "__main__":
    raise SystemExit(main())
