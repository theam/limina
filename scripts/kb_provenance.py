#!/usr/bin/env python3
"""
Provenance and staleness tracking for Limina knowledge base.

Detects:
- Findings referencing superseded hypotheses
- Decisions citing outdated findings
- Contradictions between findings
- Stale literature entries
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

try:
    import frontmatter
    HAS_FRONTMATTER = True
except ImportError:
    HAS_FRONTMATTER = False

from datetime import datetime, timedelta

ARTIFACT_ID_RE = re.compile(r"\b(?:FT|INV|IMP|RET|CR|SR|T|H|E|F|L)\d{3}\b")
META_RE = re.compile(r"^>\s+\*\*(.+?)\*\*:\s*(.+?)\s*$")


@dataclass
class StaleWarning:
    severity: str  # CRITICAL, HIGH, MEDIUM, LOW
    artifact_id: str
    message: str
    path: str


def parse_metadata(text: str) -> dict[str, str]:
    """Parse metadata from blockquote format and YAML frontmatter, merging both.

    YAML frontmatter takes priority over blockquote metadata, matching
    the validator's behavior (frontmatter is the editable source of truth).
    """
    metadata: dict[str, str] = {}

    # First pass: blockquote metadata
    for line in text.splitlines():
        match = META_RE.match(line.strip())
        if match:
            metadata[match.group(1).strip()] = match.group(2).strip()

    # Second pass: YAML frontmatter wins on conflict
    if HAS_FRONTMATTER:
        try:
            post = frontmatter.loads(text)
            for k, v in post.metadata.items():
                if v is not None and str(v) != "":
                    metadata[str(k)] = str(v)
        except Exception:
            pass

    return metadata


def collect_artifacts(kb_root: Path) -> dict[str, dict]:
    """Collect all artifacts with their metadata."""
    artifacts = {}
    dirs = {
        "H": "research/hypotheses",
        "E": "research/experiments",
        "F": "research/findings",
        "L": "research/literature",
        "T": "tasks",
        "FT": "engineering/features",
        "INV": "engineering/investigations",
        "IMP": "engineering/implementations",
        "RET": "engineering/retrospectives",
        "CR": "reports",
        "SR": "reports",
    }

    for prefix, dir_path in dirs.items():
        full_dir = kb_root / dir_path
        if not full_dir.exists():
            continue
        pattern = re.compile(rf"^{prefix}(\d{{3}})-.+\.md$")
        for path in sorted(full_dir.glob("*.md")):
            match = pattern.match(path.name)
            if not match:
                continue
            id_num = int(match.group(1))
            artifact_id = f"{prefix}{id_num:03d}"
            text = path.read_text(encoding="utf-8")
            meta = parse_metadata(text)
            artifacts[artifact_id] = {
                "id": artifact_id,
                "prefix": prefix,
                "path": str(path),
                "metadata": meta,
                "text": text,
            }

    return artifacts


def check_superseded_references(artifacts: dict) -> list[StaleWarning]:
    """Find findings that reference superseded hypotheses."""
    warnings = []

    # Build supersession chains from frontmatter
    superseded_by: dict[str, str] = {}
    for art_id, art in artifacts.items():
        supersedes = art["metadata"].get("supersedes", "")
        if supersedes and ARTIFACT_ID_RE.match(supersedes):
            superseded_by[supersedes] = art_id

    # Check findings referencing superseded artifacts
    for art_id, art in artifacts.items():
        if not art_id.startswith("F"):
            continue
        for field in ["hypothesis", "Hypothesis", "experiment", "Experiment"]:
            ref = art["metadata"].get(field, "")
            if ref in superseded_by:
                warnings.append(StaleWarning(
                    severity="HIGH",
                    artifact_id=art_id,
                    message=f"{art_id} references {ref} which was superseded by {superseded_by[ref]}. Review {art_id} for consistency.",
                    path=art["path"],
                ))

    # Check decisions referencing superseded artifacts
    return warnings


def check_rejected_hypothesis_references(artifacts: dict, kb_root: Path) -> list[StaleWarning]:
    """Find artifacts that reference rejected hypotheses without acknowledging it."""
    warnings = []
    rejected = set()

    for art_id, art in artifacts.items():
        if not art_id.startswith("H"):
            continue
        status = art["metadata"].get("status", art["metadata"].get("Status", ""))
        if status.upper() == "REJECTED":
            rejected.add(art_id)

    # Check if any decisions reference rejected hypotheses
    decisions_path = kb_root / "mission" / "DECISIONS.md"
    if not decisions_path.exists():
        decisions_path = None

    if decisions_path and decisions_path.exists():
        text = decisions_path.read_text(encoding="utf-8")
        referenced = set(ARTIFACT_ID_RE.findall(text))
        for ref in referenced & rejected:
            warnings.append(StaleWarning(
                severity="MEDIUM",
                artifact_id="DECISIONS",
                message=f"DECISIONS.md references {ref} which has status REJECTED. Verify the decision is still valid.",
                path=str(decisions_path),
            ))

    return warnings


def check_stale_literature(artifacts: dict, max_age_days: int = 180) -> list[StaleWarning]:
    """Find literature entries older than threshold."""
    warnings = []
    cutoff = datetime.now() - timedelta(days=max_age_days)

    for art_id, art in artifacts.items():
        if not art_id.startswith("L"):
            continue
        date_str = art["metadata"].get("created", art["metadata"].get("Date reviewed", ""))
        if not date_str:
            continue
        try:
            # Try common date formats
            for fmt in ["%Y-%m-%d", "%Y-%m-%dT%H:%M:%S", "%d %b %Y", "%B %d, %Y"]:
                try:
                    date = datetime.strptime(date_str, fmt)
                    if date < cutoff:
                        warnings.append(StaleWarning(
                            severity="LOW",
                            artifact_id=art_id,
                            message=f"{art_id} was reviewed on {date_str} (>{max_age_days} days ago). Consider checking for newer work.",
                            path=art["path"],
                        ))
                    break
                except ValueError:
                    continue
        except Exception:
            pass

    return warnings


def check_contradictions(artifacts: dict) -> list[StaleWarning]:
    """Find findings that may contradict each other on the same topic."""
    warnings = []
    findings_by_hypothesis: dict[str, list[str]] = {}

    for art_id, art in artifacts.items():
        if not art_id.startswith("F"):
            continue
        hypo = art["metadata"].get("hypothesis", art["metadata"].get("Hypothesis", ""))
        if hypo:
            findings_by_hypothesis.setdefault(hypo, []).append(art_id)

    # Multiple findings for the same hypothesis might indicate evolution or contradiction
    for hypo, finding_ids in findings_by_hypothesis.items():
        if len(finding_ids) > 1:
            warnings.append(StaleWarning(
                severity="MEDIUM",
                artifact_id=hypo,
                message=f"Multiple findings reference {hypo}: {', '.join(finding_ids)}. Verify they are consistent and the latest supersedes earlier ones.",
                path="",
            ))

    return warnings


def run_stale_check(kb_root: Path, max_age_days: int = 180) -> list[StaleWarning]:
    """Run all staleness checks."""
    artifacts = collect_artifacts(kb_root)
    warnings = []
    warnings.extend(check_superseded_references(artifacts))
    warnings.extend(check_rejected_hypothesis_references(artifacts, kb_root))
    warnings.extend(check_stale_literature(artifacts, max_age_days))
    warnings.extend(check_contradictions(artifacts))
    return warnings


def main() -> int:
    parser = argparse.ArgumentParser(description="Check kb/ for provenance and staleness issues.")
    parser.add_argument("--kb-root", default="./kb", help="Path to kb/ directory")
    parser.add_argument("--max-age-days", type=int, default=180, help="Max age for literature entries (days)")
    parser.add_argument("--stale-check", action="store_true", help="Run staleness checks and output warnings")
    parser.add_argument("--format", choices=("text", "json"), default="text", help="Output format")
    args = parser.parse_args()

    kb_root = Path(args.kb_root).resolve()
    if not kb_root.exists():
        print(f"KB root not found: {kb_root}", file=sys.stderr)
        return 1

    warnings = run_stale_check(kb_root, args.max_age_days)

    if not warnings:
        if args.format == "json":
            print(json.dumps({"warnings": [], "count": 0}))
        else:
            print("No staleness issues detected.")
        return 0

    if args.format == "json":
        print(json.dumps({
            "warnings": [
                {"severity": w.severity, "artifact": w.artifact_id, "message": w.message, "path": w.path}
                for w in warnings
            ],
            "count": len(warnings),
        }, indent=2))
    else:
        print(f"Found {len(warnings)} staleness warning(s):\n")
        for w in sorted(warnings, key=lambda x: {"CRITICAL": 0, "HIGH": 1, "MEDIUM": 2, "LOW": 3}.get(x.severity, 4)):
            print(f"  [{w.severity}] {w.message}")
        print()

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
