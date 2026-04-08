#!/usr/bin/env python3
"""
Provenance and staleness checks for the slim Limina research core.

Detects:
- findings referencing superseded hypotheses or experiments
- stale literature notes
- multiple findings attached to the same hypothesis
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from datetime import datetime, timedelta
from pathlib import Path

try:
    import frontmatter

    HAS_FRONTMATTER = True
except ImportError:
    HAS_FRONTMATTER = False


ARTIFACT_ID_RE = re.compile(r"\b(?:CR|SR|H|E|F|L)\d{3}\b")
META_RE = re.compile(r"^>\s+\*\*(.+?)\*\*:\s*(.+?)\s*$")

_FRONTMATTER_TO_META = {
    "hypothesis": "Hypothesis",
    "experiment": "Experiment",
    "source_type": "Type",
    "relevance": "Relevance",
    "created": "Created",
    "date_reviewed": "Date reviewed",
    "status": "Status",
}


@dataclass
class StaleWarning:
    severity: str
    artifact_id: str
    message: str
    path: str


def parse_metadata(text: str) -> dict[str, str]:
    metadata: dict[str, str] = {}
    for line in text.splitlines():
        match = META_RE.match(line.strip())
        if match:
            metadata[match.group(1).strip()] = match.group(2).strip()

    if HAS_FRONTMATTER:
        try:
            post = frontmatter.loads(text)
            for key, value in post.metadata.items():
                if value is None or str(value) == "":
                    continue
                metadata[_FRONTMATTER_TO_META.get(str(key), str(key))] = str(value)
        except Exception:
            pass

    return metadata


def collect_artifacts(kb_root: Path) -> dict[str, dict]:
    dirs = {
        "H": "research/hypotheses",
        "E": "research/experiments",
        "F": "research/findings",
        "L": "research/literature",
        "CR": "reports",
        "SR": "reports",
    }

    artifacts: dict[str, dict] = {}
    for prefix, rel_dir in dirs.items():
        directory = kb_root / rel_dir
        if not directory.exists():
            continue
        pattern = re.compile(rf"^{prefix}(\d{{3}})-.+\.md$")
        for path in directory.glob("*.md"):
            if path.name.startswith("."):
                continue
            if not pattern.match(path.name):
                continue
            artifact_id = path.name.split("-", 1)[0]
            text = path.read_text(encoding="utf-8")
            artifacts[artifact_id] = {
                "id": artifact_id,
                "path": str(path),
                "metadata": parse_metadata(text),
                "text": text,
            }
    return artifacts


def check_superseded_references(artifacts: dict[str, dict]) -> list[StaleWarning]:
    warnings: list[StaleWarning] = []
    superseded_by: dict[str, str] = {}

    for artifact_id, artifact in artifacts.items():
        supersedes = artifact["metadata"].get("supersedes", "")
        if supersedes and ARTIFACT_ID_RE.match(supersedes):
            superseded_by[supersedes] = artifact_id

    for artifact_id, artifact in artifacts.items():
        if not artifact_id.startswith("F"):
            continue
        for field in ("Hypothesis", "Experiment"):
            ref = artifact["metadata"].get(field, "")
            if ref in superseded_by:
                warnings.append(
                    StaleWarning(
                        severity="HIGH",
                        artifact_id=artifact_id,
                        message=f"{artifact_id} references {ref}, which was superseded by {superseded_by[ref]}.",
                        path=artifact["path"],
                    )
                )

    return warnings


def check_stale_literature(artifacts: dict[str, dict], max_age_days: int) -> list[StaleWarning]:
    warnings: list[StaleWarning] = []
    cutoff = datetime.now() - timedelta(days=max_age_days)

    for artifact_id, artifact in artifacts.items():
        if not artifact_id.startswith("L"):
            continue
        date_str = artifact["metadata"].get("Date reviewed", "") or artifact["metadata"].get("Created", "")
        if not date_str:
            continue
        for fmt in ("%Y-%m-%d", "%Y-%m-%dT%H:%M:%S"):
            try:
                reviewed_at = datetime.strptime(date_str, fmt)
                if reviewed_at < cutoff:
                    warnings.append(
                        StaleWarning(
                            severity="LOW",
                            artifact_id=artifact_id,
                            message=f"{artifact_id} was reviewed on {date_str}. Consider checking for newer work.",
                            path=artifact["path"],
                        )
                    )
                break
            except ValueError:
                continue

    return warnings


def check_multiple_findings_per_hypothesis(artifacts: dict[str, dict]) -> list[StaleWarning]:
    warnings: list[StaleWarning] = []
    by_hypothesis: dict[str, list[str]] = {}

    for artifact_id, artifact in artifacts.items():
        if not artifact_id.startswith("F"):
            continue
        hypothesis = artifact["metadata"].get("Hypothesis", "")
        if hypothesis:
            by_hypothesis.setdefault(hypothesis, []).append(artifact_id)

    for hypothesis, findings in by_hypothesis.items():
        if len(findings) > 1:
            warnings.append(
                StaleWarning(
                    severity="MEDIUM",
                    artifact_id=hypothesis,
                    message=f"Multiple findings reference {hypothesis}: {', '.join(sorted(findings))}.",
                    path="",
                )
            )

    return warnings


def run_checks(kb_root: Path, max_age_days: int) -> list[StaleWarning]:
    artifacts = collect_artifacts(kb_root)
    warnings: list[StaleWarning] = []
    warnings.extend(check_superseded_references(artifacts))
    warnings.extend(check_stale_literature(artifacts, max_age_days))
    warnings.extend(check_multiple_findings_per_hypothesis(artifacts))
    return warnings


def main() -> int:
    parser = argparse.ArgumentParser(description="Check Limina kb/ provenance and staleness.")
    parser.add_argument("--kb-root", default="./kb", help="Path to kb/ directory")
    parser.add_argument("--max-age-days", type=int, default=180, help="Age threshold for literature notes")
    parser.add_argument("--format", choices=("text", "json"), default="text")
    args = parser.parse_args()

    kb_root = Path(args.kb_root).resolve()
    if not kb_root.exists():
        print(f"KB root not found: {kb_root}", file=sys.stderr)
        return 1

    warnings = run_checks(kb_root, args.max_age_days)

    if args.format == "json":
        print(
            json.dumps(
                {
                    "count": len(warnings),
                    "warnings": [warning.__dict__ for warning in warnings],
                },
                indent=2,
            )
        )
        return 0

    if not warnings:
        print("No provenance or staleness issues detected.")
        return 0

    print(f"Found {len(warnings)} warning(s):")
    for warning in warnings:
        location = f" ({warning.path})" if warning.path else ""
        print(f"- [{warning.severity}] {warning.artifact_id}: {warning.message}{location}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
