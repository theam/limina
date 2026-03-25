#!/usr/bin/env python3
"""
Validate the Limina knowledge base.

The validator is intentionally read-only. It validates the core tracked
artifact model documented in README.md / CLAUDE.md / AGENTS.md.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path


CORE_ARTIFACTS = {
    "T": {"dir": Path("tasks"), "prefix": "T"},
    "H": {"dir": Path("research/hypotheses"), "prefix": "H"},
    "E": {"dir": Path("research/experiments"), "prefix": "E"},
    "F": {"dir": Path("research/findings"), "prefix": "F"},
    "L": {"dir": Path("research/literature"), "prefix": "L"},
    "FT": {"dir": Path("engineering/features"), "prefix": "FT"},
    "INV": {"dir": Path("engineering/investigations"), "prefix": "INV"},
    "IMP": {"dir": Path("engineering/implementations"), "prefix": "IMP"},
    "RET": {"dir": Path("engineering/retrospectives"), "prefix": "RET"},
    "CR": {"dir": Path("reports"), "prefix": "CR"},
    "SR": {"dir": Path("reports"), "prefix": "SR"},
}

CORE_ID_RE = re.compile(r"\b(?:FT|INV|IMP|RET|CR|SR|T|H|E|F|L)\d{3}\b")
META_RE = re.compile(r"^>\s+\*\*(.+?)\*\*:\s*(.+?)\s*$")
BACKLOG_LAST_IDS_RE = re.compile(r"^>\s+(.+)$")
FILENAME_RE = {
    prefix: re.compile(rf"^{prefix}(\d{{3}})-.+\.md$")
    for prefix in CORE_ARTIFACTS
}
HTML_COMMENT_RE = re.compile(r"<!--.*?-->", re.DOTALL)


@dataclass
class Artifact:
    prefix: str
    id_num: int
    artifact_id: str
    path: Path
    metadata: dict[str, str]


class ValidationResult:
    def __init__(self) -> None:
        self.errors: list[dict[str, str]] = []
        self.checks: dict[str, dict[str, object]] = {}

    def add_error(self, check: str, message: str, path: Path | None = None) -> None:
        error = {"check": check, "message": message}
        if path is not None:
            error["path"] = str(path)
        self.errors.append(error)

    def set_check(self, check: str, **details: object) -> None:
        self.checks[check] = details

    @property
    def ok(self) -> bool:
        return not self.errors

    def as_json(self) -> dict[str, object]:
        return {
            "ok": self.ok,
            "error_count": len(self.errors),
            "errors": self.errors,
            "checks": self.checks,
        }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Validate the kb/ artifact graph.")
    parser.add_argument(
        "--kb-root",
        default="./kb",
        help="Path to the kb/ directory (default: ./kb)",
    )
    parser.add_argument(
        "--format",
        dest="output_format",
        choices=("text", "json"),
        default="text",
        help="Output format (default: text)",
    )
    return parser.parse_args()


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def parse_metadata(text: str) -> dict[str, str]:
    metadata: dict[str, str] = {}
    for line in text.splitlines():
        match = META_RE.match(line.strip())
        if match:
            metadata[match.group(1).strip()] = match.group(2).strip()
    return metadata


def strip_comments(text: str) -> str:
    return HTML_COMMENT_RE.sub("", text)


def parse_last_ids(backlog_path: Path, result: ValidationResult) -> dict[str, int]:
    text = read_text(backlog_path)
    for line in text.splitlines():
        match = BACKLOG_LAST_IDS_RE.match(line.strip())
        if not match:
            continue
        content = match.group(1)
        if "T:" not in content:
            continue
        parsed: dict[str, int] = {}
        for part in content.split("|"):
            item = part.strip()
            if ":" not in item:
                continue
            key, raw_value = item.split(":", 1)
            key = key.strip()
            raw_value = raw_value.strip()
            if key in CORE_ARTIFACTS:
                try:
                    parsed[key] = int(raw_value)
                except ValueError:
                    result.add_error(
                        "backlog_last_ids",
                        f"Invalid last ID value for {key}: {raw_value}",
                        backlog_path,
                    )
        return parsed
    result.add_error("backlog_last_ids", "Could not find the Last IDs declaration.", backlog_path)
    return {}


def parse_backlog_tasks(backlog_path: Path, result: ValidationResult) -> dict[str, dict[str, str]]:
    tasks: dict[str, dict[str, str]] = {}
    in_tasks = False
    for raw_line in read_text(backlog_path).splitlines():
        line = raw_line.strip()
        if line == "## Tasks":
            in_tasks = True
            continue
        if not in_tasks:
            continue
        if not line:
            continue
        if line.startswith("## "):
            break
        if not line.startswith("|"):
            continue
        columns = [col.strip() for col in line.strip("|").split("|")]
        if not columns or columns[0] in {"ID", "---"}:
            continue
        if len(columns) < 6:
            result.add_error(
                "backlog_tasks",
                f"Malformed task row: {raw_line.strip()}",
                backlog_path,
            )
            continue
        task_id = columns[0]
        tasks[task_id] = {
            "Task": columns[1],
            "Status": columns[2],
            "Priority": columns[3],
            "Type": columns[4],
            "Linked": columns[5],
        }
    return tasks


def collect_artifacts(kb_root: Path, result: ValidationResult) -> dict[str, dict[str, Artifact]]:
    collected: dict[str, dict[str, Artifact]] = {prefix: {} for prefix in CORE_ARTIFACTS}
    duplicates: list[tuple[str, Path, Path]] = []

    for prefix, config in CORE_ARTIFACTS.items():
        artifact_dir = kb_root / config["dir"]
        if not artifact_dir.exists():
            result.add_error("filesystem", f"Missing artifact directory for {prefix}: {artifact_dir}", artifact_dir)
            continue
        for path in sorted(artifact_dir.glob("*.md")):
            match = FILENAME_RE[prefix].match(path.name)
            if not match:
                if config["dir"] == Path("reports") and not path.name.startswith(prefix):
                    continue
                result.add_error("filenames", f"Malformed filename for {prefix}: {path.name}", path)
                continue
            id_num = int(match.group(1))
            artifact_id = f"{prefix}{id_num:03d}"
            artifact = Artifact(
                prefix=prefix,
                id_num=id_num,
                artifact_id=artifact_id,
                path=path,
                metadata=parse_metadata(read_text(path)),
            )
            if artifact_id in collected[prefix]:
                duplicates.append((artifact_id, collected[prefix][artifact_id].path, path))
            else:
                collected[prefix][artifact_id] = artifact

    for artifact_id, first, second in duplicates:
        result.add_error("duplicate_ids", f"Duplicate artifact ID {artifact_id}.", first)
        result.add_error("duplicate_ids", f"Duplicate artifact ID {artifact_id}.", second)

    return collected


def check_required_metadata(artifacts: dict[str, dict[str, Artifact]], result: ValidationResult) -> None:
    required = {
        "T": ["Status", "Priority", "Type"],
        "H": ["Status", "Task"],
        "E": ["Status", "Hypothesis", "Task"],
        "F": ["Task", "Hypothesis", "Experiment", "Impact"],
        "L": ["Task", "Type", "Relevance"],
        "FT": ["Task", "Status"],
        "INV": ["Task", "Feature", "Status"],
        "IMP": ["Task", "Feature", "Investigation", "Status"],
        "RET": ["Task", "Feature", "Implementation"],
        "CR": ["Date", "Target", "Requested by", "Reviewer"],
        "SR": ["Date", "Scope", "Challenge Review"],
    }
    for prefix, by_id in artifacts.items():
        for artifact in by_id.values():
            missing = [field for field in required[prefix] if field not in artifact.metadata]
            if missing:
                result.add_error(
                    "required_metadata",
                    f"{artifact.artifact_id} is missing metadata fields: {', '.join(missing)}",
                    artifact.path,
                )


def check_gaps_and_last_ids(
    artifacts: dict[str, dict[str, Artifact]],
    last_ids: dict[str, int],
    result: ValidationResult,
) -> None:
    summary: dict[str, dict[str, int]] = {}
    for prefix, by_id in artifacts.items():
        numbers = sorted(artifact.id_num for artifact in by_id.values())
        highest = numbers[-1] if numbers else 0
        declared = last_ids.get(prefix, 0)
        if prefix not in last_ids:
            result.add_error("backlog_last_ids", f"BACKLOG.md is missing a Last IDs entry for {prefix}.")
        if declared != highest:
            result.add_error(
                "backlog_last_ids",
                f"BACKLOG.md declares {prefix}: {declared:03d}, but highest existing file is {highest:03d}.",
            )
        expected = set(range(1, declared + 1))
        actual = {artifact.id_num for artifact in by_id.values()}
        for missing in sorted(expected - actual):
            result.add_error(
                "id_gaps",
                f"Missing {prefix}{missing:03d} while BACKLOG.md declares {prefix}: {declared:03d}.",
            )
        summary[prefix] = {"declared": declared, "highest_existing": highest, "count": len(actual)}
    result.set_check("last_ids", summary=summary)


def parse_task_title(text: str) -> str:
    first = text.splitlines()[0].strip()
    if "—" in first:
        return first.split("—", 1)[1].strip()
    return first


def check_backlog_task_sync(
    backlog_tasks: dict[str, dict[str, str]],
    artifacts: dict[str, dict[str, Artifact]],
    result: ValidationResult,
) -> None:
    task_files = artifacts["T"]
    for task_id, task_artifact in task_files.items():
        if task_id not in backlog_tasks:
            result.add_error("backlog_tasks", f"{task_id} exists as a task file but has no BACKLOG.md row.", task_artifact.path)
            continue
        row = backlog_tasks[task_id]
        title = parse_task_title(read_text(task_artifact.path))
        if row["Task"] != title:
            result.add_error(
                "backlog_tasks",
                f"{task_id} title mismatch between task file and BACKLOG.md.",
                task_artifact.path,
            )
        for field in ("Status", "Priority", "Type"):
            if row[field] != task_artifact.metadata.get(field, ""):
                result.add_error(
                    "backlog_tasks",
                    f"{task_id} {field.lower()} mismatch between task file and BACKLOG.md.",
                    task_artifact.path,
                )
    for task_id in backlog_tasks:
        if task_id not in task_files:
            result.add_error("backlog_tasks", f"{task_id} exists in BACKLOG.md but has no task file.")
    result.set_check(
        "backlog_tasks",
        backlog_rows=len(backlog_tasks),
        task_files=len(task_files),
    )


def extract_ids_from_text(text: str) -> set[str]:
    return set(CORE_ID_RE.findall(strip_comments(text)))


def check_index_coverage(kb_root: Path, artifacts: dict[str, dict[str, Artifact]], result: ValidationResult) -> None:
    index_path = kb_root / "INDEX.md"
    if not index_path.exists():
        result.add_error("index", "Missing kb/INDEX.md.", index_path)
        return
    index_ids = extract_ids_from_text(read_text(index_path))
    existing_ids = {
        artifact.artifact_id
        for prefix, by_id in artifacts.items()
        for artifact in by_id.values()
    }
    for artifact_id in sorted(existing_ids - index_ids):
        artifact = next(
            artifact
            for by_id in artifacts.values()
            for artifact in by_id.values()
            if artifact.artifact_id == artifact_id
        )
        result.add_error("index", f"{artifact_id} exists but is not referenced in INDEX.md.", artifact.path)
    for artifact_id in sorted(index_ids - existing_ids):
        result.add_error("index", f"INDEX.md references {artifact_id}, but no matching core artifact file exists.", index_path)
    result.set_check("index", referenced=len(index_ids), existing=len(existing_ids))


def validate_ref(
    artifact: Artifact,
    field: str,
    target_prefix: str,
    artifacts: dict[str, dict[str, Artifact]],
    result: ValidationResult,
) -> None:
    raw_value = artifact.metadata.get(field)
    if raw_value is None:
        return
    expected_re = re.compile(rf"^{target_prefix}\d{{3}}$")
    if not expected_re.match(raw_value):
        result.add_error(
            "traceability",
            f"{artifact.artifact_id} has invalid {field} reference: {raw_value}",
            artifact.path,
        )
        return
    if raw_value not in artifacts[target_prefix]:
        result.add_error(
            "traceability",
            f"{artifact.artifact_id} references missing {field}: {raw_value}",
            artifact.path,
        )


def check_traceability(artifacts: dict[str, dict[str, Artifact]], result: ValidationResult) -> None:
    for artifact in artifacts["H"].values():
        validate_ref(artifact, "Task", "T", artifacts, result)
    for artifact in artifacts["E"].values():
        validate_ref(artifact, "Task", "T", artifacts, result)
        validate_ref(artifact, "Hypothesis", "H", artifacts, result)
    for artifact in artifacts["F"].values():
        validate_ref(artifact, "Task", "T", artifacts, result)
        validate_ref(artifact, "Hypothesis", "H", artifacts, result)
        validate_ref(artifact, "Experiment", "E", artifacts, result)
    for artifact in artifacts["L"].values():
        validate_ref(artifact, "Task", "T", artifacts, result)
    for artifact in artifacts["FT"].values():
        validate_ref(artifact, "Task", "T", artifacts, result)
    for artifact in artifacts["INV"].values():
        validate_ref(artifact, "Task", "T", artifacts, result)
        validate_ref(artifact, "Feature", "FT", artifacts, result)
    for artifact in artifacts["IMP"].values():
        validate_ref(artifact, "Task", "T", artifacts, result)
        validate_ref(artifact, "Feature", "FT", artifacts, result)
        validate_ref(artifact, "Investigation", "INV", artifacts, result)
    for artifact in artifacts["RET"].values():
        validate_ref(artifact, "Task", "T", artifacts, result)
        validate_ref(artifact, "Feature", "FT", artifacts, result)
        validate_ref(artifact, "Implementation", "IMP", artifacts, result)
    for artifact in artifacts["SR"].values():
        validate_ref(artifact, "Challenge Review", "CR", artifacts, result)
    result.set_check(
        "traceability",
        checked=sum(len(by_id) for by_id in artifacts.values()),
    )


def validate_kb(kb_root: Path) -> ValidationResult:
    result = ValidationResult()
    backlog_path = kb_root / "mission" / "BACKLOG.md"
    if not backlog_path.exists():
        result.add_error("filesystem", "Missing kb/mission/BACKLOG.md.", backlog_path)
        return result

    last_ids = parse_last_ids(backlog_path, result)
    backlog_tasks = parse_backlog_tasks(backlog_path, result)
    artifacts = collect_artifacts(kb_root, result)
    check_required_metadata(artifacts, result)
    check_gaps_and_last_ids(artifacts, last_ids, result)
    check_backlog_task_sync(backlog_tasks, artifacts, result)
    check_index_coverage(kb_root, artifacts, result)
    check_traceability(artifacts, result)
    return result


def render_text(result: ValidationResult) -> str:
    if result.ok:
        return "KB validation passed. No errors found."
    grouped: dict[str, list[dict[str, str]]] = defaultdict(list)
    for error in result.errors:
        grouped[error["check"]].append(error)

    lines = [f"KB validation failed with {len(result.errors)} error(s)."]
    for check in sorted(grouped):
        lines.append(f"\n[{check}]")
        for error in grouped[check]:
            line = f"- {error['message']}"
            if "path" in error:
                line += f" ({error['path']})"
            lines.append(line)
    return "\n".join(lines)


def main() -> int:
    args = parse_args()
    kb_root = Path(args.kb_root).resolve()
    result = validate_kb(kb_root)
    if args.output_format == "json":
        print(json.dumps(result.as_json(), indent=2))
    else:
        print(render_text(result))
    return 0 if result.ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
