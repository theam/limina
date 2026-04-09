#!/usr/bin/env python3
"""Create a Limina core artifact from a template and maintain first backlinks."""

from __future__ import annotations

import argparse
import os
import re
import sys
from datetime import date
from pathlib import Path


ARTIFACTS = {
    "H": {
        "template": Path("templates/hypothesis.md"),
        "directory": Path("kb/research/hypotheses"),
    },
    "E": {
        "template": Path("templates/experiment.md"),
        "directory": Path("kb/research/experiments"),
    },
    "F": {
        "template": Path("templates/finding.md"),
        "directory": Path("kb/research/findings"),
    },
    "L": {
        "template": Path("templates/literature.md"),
        "directory": Path("kb/research/literature"),
    },
    "CR": {
        "template": Path("templates/challenge-review.md"),
        "directory": Path("kb/reports"),
    },
    "SR": {
        "template": Path("templates/strategic-review.md"),
        "directory": Path("kb/reports"),
    },
}

SPECIAL_NOTES = {
    "ACTIVE": Path("kb/ACTIVE.md"),
    "CHALLENGE": Path("kb/mission/CHALLENGE.md"),
    "DASHBOARD": Path("kb/DASHBOARD.md"),
}

if os.environ.get("LIMINA_TELEMETRY_INTERNAL") != "1":
    try:
        from telemetry import emit_event as telemetry_emit_event
        from telemetry import ensure_consent as telemetry_ensure_consent
    except Exception:  # pragma: no cover - telemetry must not block artifact creation
        telemetry_emit_event = None
        telemetry_ensure_consent = None
else:  # pragma: no cover - internal telemetry calls skip recursion
    telemetry_emit_event = None
    telemetry_ensure_consent = None


def maybe_prompt_telemetry() -> None:
    if telemetry_ensure_consent is None:
        return
    try:
        telemetry_ensure_consent("kb_new_artifact")
    except Exception:
        return


def maybe_emit_artifact_created(prefix: str) -> None:
    if telemetry_emit_event is None:
        return
    try:
        telemetry_emit_event(
            "limina_artifact_created",
            emitter="kb_new_artifact",
            properties={"artifact_type": prefix},
        )
    except Exception:
        return


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Create a Limina research artifact and wire initial backlinks.")
    parser.add_argument("prefix", choices=sorted(ARTIFACTS), help="Artifact prefix to create")
    parser.add_argument("title", help="Human-readable title")
    parser.add_argument("--project-root", default=".", help="Project root containing kb/ and templates/")
    parser.add_argument("--hypothesis", help="Parent hypothesis ID (required for E and F)")
    parser.add_argument("--experiment", help="Parent experiment ID (required for F)")
    parser.add_argument("--challenge-review", help="Parent challenge review ID (required for SR)")
    parser.add_argument("--target", help="Reviewed target text (required for CR)")
    parser.add_argument("--target-id", help="Reviewed target artifact ID when the CR targets a note")
    parser.add_argument("--scope", help="Strategic review scope (required for SR)")
    parser.add_argument("--requested-by", default="user", help="Requested-by value for CR")
    parser.add_argument("--reviewer", default="Devil's Advocate", help="Reviewer value for CR")
    parser.add_argument("--impact", default="HIGH", help="Impact value for F")
    parser.add_argument("--source-type", default="PAPER", help="Source type value for L")
    parser.add_argument("--relevance", default="HIGH", help="Relevance value for L")
    parser.add_argument("--source-url", default="", help="Source URL value for L")
    parser.add_argument(
        "--link",
        action="append",
        default=[],
        help="Additional note to link from the new artifact (ID alias or note stem). Can be repeated.",
    )
    return parser.parse_args()


def slugify(title: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-")
    return slug or "untitled"


def next_id(prefix: str, project_root: Path) -> str:
    directory = project_root / ARTIFACTS[prefix]["directory"]
    directory.mkdir(parents=True, exist_ok=True)
    pattern = re.compile(rf"^{prefix}(\d{{3}})-.+\.md$")
    highest = 0
    for path in directory.glob("*.md"):
        match = pattern.match(path.name)
        if match:
            highest = max(highest, int(match.group(1)))
    return f"{prefix}{highest + 1:03d}"


def resolve_note_path(note_ref: str, project_root: Path) -> Path | None:
    note_ref = note_ref.strip()
    if note_ref in SPECIAL_NOTES:
        path = project_root / SPECIAL_NOTES[note_ref]
        return path if path.exists() else None

    if re.fullmatch(r"(?:CR|SR|H|E|F|L)\d{3}", note_ref):
        prefix = re.match(r"[A-Z]+", note_ref).group(0)
        directory = project_root / ARTIFACTS[prefix]["directory"]
        matches = sorted(directory.glob(f"{note_ref}-*.md"))
        return matches[0] if matches else None

    lesson_path = project_root / "kb/lessons" / f"{note_ref}.md"
    if lesson_path.exists():
        return lesson_path

    matches = list((project_root / "kb").glob(f"**/{note_ref}.md"))
    return matches[0] if matches else None


def require_existing_note(note_ref: str, project_root: Path, flag_name: str) -> None:
    if resolve_note_path(note_ref, project_root) is None:
        raise SystemExit(f"{flag_name} references missing note: {note_ref}")


def build_links(prefix: str, args: argparse.Namespace) -> list[tuple[str, str]]:
    links: list[tuple[str, str]] = [("Mission", "CHALLENGE"), ("Active State", "ACTIVE")]

    if prefix == "E":
        links.append(("Parent Hypothesis", args.hypothesis))
    elif prefix == "F":
        links.append(("Parent Hypothesis", args.hypothesis))
        links.append(("Parent Experiment", args.experiment))
    elif prefix == "CR" and args.target_id:
        links.append(("Target Artifact", args.target_id))
    elif prefix == "SR":
        links.append(("Challenge Review", args.challenge_review))

    for extra in args.link:
        links.append(("Related", extra))

    seen: set[str] = set()
    deduped: list[tuple[str, str]] = []
    for label, target in links:
        if not target or target in seen:
            continue
        seen.add(target)
        deduped.append((label, target))
    return deduped


def render_links_block(links: list[tuple[str, str]]) -> str:
    return "\n".join(f"- {label}: [[{target}]]" for label, target in links)


def add_link_to_note(path: Path, label: str, target: str) -> None:
    if not path.exists():
        return

    text = path.read_text(encoding="utf-8")
    wikilink = f"[[{target}]]"
    lines = text.splitlines()

    start = None
    end = None
    for idx, line in enumerate(lines):
        if line.strip() == "## Links":
            start = idx
            break

    if start is None:
        if lines and lines[-1] != "":
            lines.append("")
        lines.extend(["## Links", "", f"- {label}: {wikilink}"])
        path.write_text("\n".join(lines) + "\n", encoding="utf-8")
        return

    for idx in range(start + 1, len(lines)):
        if lines[idx].startswith("## "):
            end = idx
            break
    if end is None:
        end = len(lines)

    section = lines[start:end]
    if any(wikilink in line for line in section):
        return

    insert_at = end
    if insert_at > start + 1 and lines[insert_at - 1] == "":
        insert_at -= 1
    lines.insert(insert_at, f"- {label}: {wikilink}")
    if insert_at == start + 1:
        lines.insert(insert_at, "")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def render_template(template_text: str, replacements: dict[str, str]) -> str:
    output = template_text
    for key, value in replacements.items():
        output = output.replace(f"{{{key}}}", value)
    return output


def backlink_targets(prefix: str, artifact_id: str, args: argparse.Namespace) -> list[tuple[str, str]]:
    backlinks: list[tuple[str, str]] = []
    if prefix == "E":
        backlinks.append((args.hypothesis, "Experiment"))
    elif prefix == "F":
        backlinks.append((args.hypothesis, "Finding"))
        backlinks.append((args.experiment, "Finding"))
    elif prefix == "CR" and args.target_id:
        backlinks.append((args.target_id, "Challenge Review"))
    elif prefix == "SR":
        backlinks.append((args.challenge_review, "Strategic Review"))

    for extra in args.link:
        backlinks.append((extra, "Related"))
    return backlinks


def main() -> int:
    args = parse_args()
    maybe_prompt_telemetry()
    project_root = Path(args.project_root).resolve()
    prefix = args.prefix

    if prefix in {"E", "F"} and not args.hypothesis:
        raise SystemExit("--hypothesis is required for E and F")
    if prefix == "F" and not args.experiment:
        raise SystemExit("--experiment is required for F")
    if prefix == "CR" and not args.target:
        raise SystemExit("--target is required for CR")
    if prefix == "SR" and not args.scope:
        raise SystemExit("--scope is required for SR")
    if prefix == "SR" and not args.challenge_review:
        raise SystemExit("--challenge-review is required for SR")

    for ref, flag in (
        (args.hypothesis, "--hypothesis"),
        (args.experiment, "--experiment"),
        (args.challenge_review, "--challenge-review"),
        (args.target_id, "--target-id"),
    ):
        if ref:
            require_existing_note(ref, project_root, flag)
    for ref in args.link:
        require_existing_note(ref, project_root, "--link")

    artifact_id = next_id(prefix, project_root)
    date_str = date.today().isoformat()
    slug = slugify(args.title)
    artifact_path = project_root / ARTIFACTS[prefix]["directory"] / f"{artifact_id}-{slug}.md"
    template_path = project_root / ARTIFACTS[prefix]["template"]
    template_text = template_path.read_text(encoding="utf-8")

    links = build_links(prefix, args)
    replacements = {
        "ARTIFACT_ID": artifact_id,
        "DATE": date_str,
        "TITLE": args.title,
        "HYPOTHESIS_ID": args.hypothesis or "",
        "EXPERIMENT_ID": args.experiment or "",
        "CHALLENGE_REVIEW_ID": args.challenge_review or "",
        "TARGET": args.target or "",
        "TARGET_ID": args.target_id or "",
        "TARGET_ID_DISPLAY": f"[[{args.target_id}]]" if args.target_id else "N/A",
        "REQUESTED_BY": args.requested_by,
        "REVIEWER": args.reviewer,
        "SCOPE": args.scope or "",
        "SOURCE_TYPE": args.source_type,
        "RELEVANCE": args.relevance,
        "SOURCE_URL": args.source_url,
        "IMPACT": args.impact,
        "LINKS_BLOCK": render_links_block(links),
    }

    artifact_text = render_template(template_text, replacements)
    artifact_path.parent.mkdir(parents=True, exist_ok=True)
    artifact_path.write_text(artifact_text, encoding="utf-8")

    for target, label in backlink_targets(prefix, artifact_id, args):
        target_path = resolve_note_path(target, project_root)
        if target_path is not None:
            add_link_to_note(target_path, label, artifact_id)

    maybe_emit_artifact_created(prefix)
    print(artifact_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
