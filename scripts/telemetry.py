#!/usr/bin/env python3
"""Consent-first telemetry client for Limina."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

PROJECT_ROOT = Path(__file__).resolve().parent.parent
CONTRACT_PATH = PROJECT_ROOT / "telemetry" / "contract.v1.json"
VERSION_PATH = PROJECT_ROOT / "VERSION"
DEFAULT_LIMINA_HOME = Path(os.environ.get("LIMINA_HOME", "~/.limina")).expanduser()
STATE_DIR = DEFAULT_LIMINA_HOME / "telemetry"
CONFIG_PATH = STATE_DIR / "config.json"
OUTBOX_PATH = STATE_DIR / "outbox.jsonl"
SESSION_PATH = STATE_DIR / "session.json"
SESSION_IDLE_LIMIT = timedelta(hours=8)
WRITE_TOKEN_SAFETY_MARGIN = timedelta(seconds=30)
VERSION_RE = re.compile(r"^\d+\.\d+\.\d+(?:[-+][A-Za-z0-9._-]+)?$")
FRONTMATTER_LINE_RE = re.compile(r"^([A-Za-z0-9_-]+):\s*(.*)$")
META_LINE_RE = re.compile(r"^>\s+\*\*(.+?)\*\*:\s*(.+?)\s*$")
ARTIFACT_FILE_RE = re.compile(r"^(CR|SR|H|E|F|L)\d{3}-.*\.md$")


class TelemetryError(RuntimeError):
    """Raised when an event does not satisfy the privacy contract."""


def utc_now() -> datetime:
    return datetime.now(UTC)


def utc_iso(dt: datetime | None = None) -> str:
    value = dt or utc_now()
    return value.replace(microsecond=0).isoformat().replace("+00:00", "Z")


def parse_utc(value: str | None) -> datetime | None:
    if not value:
        return None
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None


def read_version() -> str:
    version = VERSION_PATH.read_text(encoding="utf-8").strip()
    if not VERSION_RE.fullmatch(version):
        raise TelemetryError(f"Invalid VERSION value: {version!r}")
    return version


def version_major(version: str) -> int:
    return int(version.split(".", 1)[0])


def load_contract() -> dict[str, Any]:
    return json.loads(CONTRACT_PATH.read_text(encoding="utf-8"))


def ensure_state_dir() -> None:
    STATE_DIR.mkdir(parents=True, exist_ok=True)


def atomic_write_json(path: Path, payload: dict[str, Any]) -> None:
    ensure_state_dir()
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=path.parent, delete=False) as handle:
        json.dump(payload, handle, indent=2, sort_keys=True)
        handle.write("\n")
        temp_path = Path(handle.name)
    temp_path.replace(path)


def load_json(path: Path, default: dict[str, Any]) -> dict[str, Any]:
    if not path.exists():
        return dict(default)
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return dict(default)
    if not isinstance(data, dict):
        return dict(default)
    merged = dict(default)
    merged.update(data)
    return merged


def default_config() -> dict[str, Any]:
    contract = load_contract()
    return {
        "schema_version": contract["schema_version"],
        "relay_url": os.environ.get("LIMINA_TELEMETRY_URL", contract["default_relay_url"]),
        "consent_state": "unset",
        "prompt_suppressed_until": "",
        "prompt_suppressed_major": 0,
        "install_id": "",
        "install_token": "",
        "created_at": utc_iso(),
        "updated_at": utc_iso(),
    }


def load_config() -> dict[str, Any]:
    return load_json(CONFIG_PATH, default_config())


def save_config(config: dict[str, Any]) -> None:
    config["updated_at"] = utc_iso()
    atomic_write_json(CONFIG_PATH, config)


def default_session() -> dict[str, Any]:
    return {
        "session_uuid": "",
        "runtime_family": "",
        "opened_at": "",
        "last_activity_at": "",
        "sequence": 0,
        "write_token": "",
        "write_token_expires_at": "",
    }


def load_session() -> dict[str, Any]:
    return load_json(SESSION_PATH, default_session())


def save_session(session: dict[str, Any]) -> None:
    atomic_write_json(SESSION_PATH, session)


def delete_path(path: Path) -> None:
    if path.exists():
        path.unlink()


def read_outbox() -> list[dict[str, Any]]:
    if not OUTBOX_PATH.exists():
        return []
    events: list[dict[str, Any]] = []
    for line in OUTBOX_PATH.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(event, dict):
            events.append(event)
    return events


def write_outbox(events: list[dict[str, Any]]) -> None:
    ensure_state_dir()
    if not events:
        delete_path(OUTBOX_PATH)
        return
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=OUTBOX_PATH.parent, delete=False) as handle:
        for event in events:
            handle.write(json.dumps(event, sort_keys=True))
            handle.write("\n")
        temp_path = Path(handle.name)
    temp_path.replace(OUTBOX_PATH)


def append_outbox(event: dict[str, Any]) -> None:
    ensure_state_dir()
    with OUTBOX_PATH.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(event, sort_keys=True))
        handle.write("\n")


def consent_enabled(config: dict[str, Any]) -> bool:
    return config.get("consent_state") in {"anonymous", "community"}


def detect_runtime_family(explicit: str | None = None) -> str:
    if explicit:
        return explicit
    env = os.environ
    if any(key in env for key in ("CODEX_THREAD_ID", "CODEX_SHELL", "CODEX_HOME")):
        return "codex"
    if any(key in env for key in ("CLAUDE_HOME", "CLAUDECODE", "CLAUDE_CODE")):
        return "claude"
    if any(key in env for key in ("OPENCODE_HOME", "OPENCODE")):
        return "opencode"
    if (Path.home() / ".codex").exists() and not (Path.home() / ".claude").exists():
        return "codex"
    if (Path.home() / ".claude").exists():
        return "claude"
    return "codex"


def interactive_prompt_allowed() -> bool:
    if os.environ.get("CI") or os.environ.get("GITHUB_ACTIONS"):
        return False
    return sys.stdin.isatty() and sys.stdout.isatty()


def should_prompt(config: dict[str, Any]) -> bool:
    consent_state = config.get("consent_state", "unset")
    if consent_state in {"anonymous", "community", "declined"}:
        return False
    if not interactive_prompt_allowed():
        return False

    current_major = version_major(read_version())
    suppressed_major = int(config.get("prompt_suppressed_major") or 0)
    if suppressed_major and suppressed_major != current_major:
        return True

    suppressed_until = parse_utc(config.get("prompt_suppressed_until"))
    return suppressed_until is None or suppressed_until <= utc_now()


def clear_local_state(config: dict[str, Any], *, consent_state: str) -> dict[str, Any]:
    updated = dict(config)
    updated["consent_state"] = consent_state
    updated["prompt_suppressed_until"] = ""
    updated["prompt_suppressed_major"] = 0
    updated["install_id"] = ""
    updated["install_token"] = ""
    save_config(updated)
    delete_path(OUTBOX_PATH)
    delete_path(SESSION_PATH)
    return updated


def prompt_for_consent() -> str:
    prompt = (
        "Help improve Limina?\n\n"
        "Share anonymous usage data to help improve the product.\n"
        "This never includes your code, prompts, repo names, file paths,\n"
        "or knowledge-base content.\n\n"
        "1) Share Anonymous Usage\n"
        "2) Not Now\n"
        "3) Never Ask Again\n"
    )
    print(prompt, flush=True)
    mapping = {
        "1": "anonymous",
        "2": "not_now",
        "3": "declined",
    }
    while True:
        answer = input("> ").strip()
        if answer in mapping:
            return mapping[answer]
        print("Choose 1, 2, or 3.", flush=True)


def apply_consent_choice(
    choice: str,
    *,
    runtime_family: str | None = None,
    emitter: str = "manual",
    flush: bool = False,
) -> dict[str, Any]:
    config = load_config()
    current_major = version_major(read_version())

    if choice == "not_now":
        config["consent_state"] = "unset"
        config["prompt_suppressed_until"] = utc_iso(utc_now() + timedelta(days=30))
        config["prompt_suppressed_major"] = current_major
        save_config(config)
        return config

    if choice == "declined":
        return clear_local_state(config, consent_state="declined")

    if choice not in {"anonymous", "community"}:
        raise TelemetryError(f"Unsupported consent choice: {choice}")

    config["consent_state"] = choice
    config["prompt_suppressed_until"] = ""
    config["prompt_suppressed_major"] = 0
    config.setdefault("install_id", "")
    if not config["install_id"]:
        config["install_id"] = str(uuid.uuid4())
    save_config(config)

    emit_event(
        "limina_telemetry_consent_granted",
        runtime_family=runtime_family,
        emitter=emitter,
        properties={},
        flush=flush,
    )
    return config


def ensure_consent(source: str = "manual", runtime_family: str | None = None) -> bool:
    config = load_config()
    if consent_enabled(config):
        return True
    if not should_prompt(config):
        return False
    choice = prompt_for_consent()
    apply_consent_choice(choice, runtime_family=runtime_family, emitter=source, flush=True)
    return choice in {"anonymous", "community"}


def normalize_scalar(spec: dict[str, Any], value: Any) -> Any:
    value_type = spec["type"]
    if value_type == "boolean":
        if isinstance(value, bool):
            return value
        if isinstance(value, str):
            lowered = value.strip().lower()
            if lowered == "true":
                return True
            if lowered == "false":
                return False
        raise TelemetryError(f"Expected boolean, got {value!r}")

    if value_type == "integer":
        if isinstance(value, bool):
            raise TelemetryError(f"Expected integer, got boolean {value!r}")
        if isinstance(value, int):
            parsed = value
        elif isinstance(value, str) and re.fullmatch(r"-?\d+", value.strip()):
            parsed = int(value.strip())
        else:
            raise TelemetryError(f"Expected integer, got {value!r}")
        minimum = spec.get("min")
        maximum = spec.get("max")
        const = spec.get("const")
        if const is not None and parsed != const:
            raise TelemetryError(f"Expected integer constant {const}, got {parsed}")
        if minimum is not None and parsed < minimum:
            raise TelemetryError(f"Integer {parsed} is smaller than {minimum}")
        if maximum is not None and parsed > maximum:
            raise TelemetryError(f"Integer {parsed} is larger than {maximum}")
        return parsed

    if value_type == "enum":
        if not isinstance(value, str):
            raise TelemetryError(f"Expected enum string, got {value!r}")
        if value not in spec["values"]:
            raise TelemetryError(f"Unsupported enum value {value!r}")
        return value

    if value_type == "version":
        if not isinstance(value, str) or not VERSION_RE.fullmatch(value):
            raise TelemetryError(f"Invalid version string {value!r}")
        return value

    raise TelemetryError(f"Unknown property type {value_type!r}")


def validate_properties(event_name: str, properties: dict[str, Any]) -> dict[str, Any]:
    contract = load_contract()
    event_spec = contract["events"].get(event_name)
    if event_spec is None:
        raise TelemetryError(f"Unknown event: {event_name}")

    allowed_names = set(event_spec["required_properties"]) | set(event_spec["optional_properties"])
    missing = [name for name in event_spec["required_properties"] if name not in properties]
    if missing:
        raise TelemetryError(f"Missing required properties for {event_name}: {', '.join(sorted(missing))}")

    unknown = sorted(set(properties) - allowed_names)
    if unknown:
        raise TelemetryError(f"Unknown properties for {event_name}: {', '.join(unknown)}")

    normalized: dict[str, Any] = {}
    for name in allowed_names:
        if name not in properties:
            continue
        spec = contract["property_specs"][name]
        normalized[name] = normalize_scalar(spec, properties[name])
    return normalized


def duration_bucket_for(started_at: str | None) -> str:
    dt = parse_utc(started_at)
    if dt is None:
        return "lt_1s"
    seconds = max((utc_now() - dt).total_seconds(), 0)
    if seconds < 1:
        return "lt_1s"
    if seconds < 5:
        return "1s_to_5s"
    if seconds < 30:
        return "5s_to_30s"
    if seconds < 120:
        return "30s_to_2m"
    if seconds < 600:
        return "2m_to_10m"
    if seconds < 3600:
        return "10m_to_1h"
    return "gte_1h"


def session_is_stale(session: dict[str, Any], runtime_family: str) -> bool:
    if not session.get("session_uuid"):
        return True
    if session.get("runtime_family") != runtime_family:
        return True
    last_activity = parse_utc(session.get("last_activity_at"))
    return last_activity is None or utc_now() - last_activity > SESSION_IDLE_LIMIT


def create_local_session(runtime_family: str) -> dict[str, Any]:
    session = load_session()
    session["session_uuid"] = str(uuid.uuid4())
    session["runtime_family"] = runtime_family
    session["opened_at"] = utc_iso()
    session["last_activity_at"] = session["opened_at"]
    session["sequence"] = 0
    save_session(session)
    return session


def ensure_local_session(runtime_family: str, *, rotate: bool = False) -> dict[str, Any]:
    session = load_session()
    if rotate or session_is_stale(session, runtime_family):
        return create_local_session(runtime_family)
    return session


def next_sequence(session: dict[str, Any]) -> int:
    session["sequence"] = int(session.get("sequence", 0)) + 1
    session["last_activity_at"] = utc_iso()
    save_session(session)
    return int(session["sequence"])


def queue_event(
    event_name: str,
    *,
    runtime_family: str | None = None,
    emitter: str,
    properties: dict[str, Any] | None = None,
    flush: bool = False,
    rotate_session: bool = False,
) -> bool:
    config = load_config()
    if not consent_enabled(config):
        return False

    runtime_value = runtime_family or detect_runtime_family()
    session = ensure_local_session(runtime_value, rotate=rotate_session)
    merged: dict[str, Any] = {
        "schema_version": load_contract()["schema_version"],
        "limina_version": read_version(),
        "event_name": event_name,
        "consent_tier": config["consent_state"],
        "emitter": emitter,
    }
    if runtime_value:
        merged["runtime_family"] = runtime_value
    if properties:
        merged.update(properties)
    if event_name == "limina_session_completed" and "duration_bucket" not in merged:
        merged["duration_bucket"] = duration_bucket_for(session.get("opened_at"))

    normalized = validate_properties(event_name, merged)
    row = {
        "event_uuid": str(uuid.uuid4()),
        "event": event_name,
        "occurred_at": utc_iso(),
        "session_uuid": session["session_uuid"],
        "seq": next_sequence(session),
        "properties": normalized,
    }
    append_outbox(row)
    if event_name == "limina_session_completed":
        delete_path(SESSION_PATH)
    if flush:
        flush_outbox()
    return True


def emit_event(
    event_name: str,
    *,
    runtime_family: str | None = None,
    emitter: str = "manual",
    properties: dict[str, Any] | None = None,
    flush: bool = False,
    rotate_session: bool = False,
) -> bool:
    return queue_event(
        event_name,
        runtime_family=runtime_family,
        emitter=emitter,
        properties=properties,
        flush=flush,
        rotate_session=rotate_session,
    )


def http_json(
    url: str,
    *,
    method: str = "POST",
    payload: dict[str, Any] | None = None,
    headers: dict[str, str] | None = None,
    timeout: int = 10,
) -> tuple[int | None, dict[str, Any] | None]:
    request_headers = {"Content-Type": "application/json"}
    if headers:
        request_headers.update(headers)
    data = None
    if payload is not None:
        data = json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(url, data=data, method=method, headers=request_headers)
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            raw = response.read().decode("utf-8")
            parsed = json.loads(raw) if raw else {}
            if not isinstance(parsed, dict):
                parsed = {}
            return response.status, parsed
    except urllib.error.HTTPError as exc:
        try:
            raw = exc.read().decode("utf-8")
            parsed = json.loads(raw) if raw else {}
        except Exception:
            parsed = {}
        return exc.code, parsed if isinstance(parsed, dict) else {}
    except (urllib.error.URLError, TimeoutError, OSError):
        return None, None


def solve_pow(challenge_id: str, salt: str, difficulty: int) -> int:
    prefix = "0" * difficulty
    nonce = 0
    while True:
        digest = hashlib.sha256(f"{challenge_id}:{salt}:{nonce}".encode("utf-8")).hexdigest()
        if digest.startswith(prefix):
            return nonce
        nonce += 1


def register_install(config: dict[str, Any]) -> dict[str, Any]:
    if not consent_enabled(config):
        return config
    if config.get("install_token"):
        return config
    if not config.get("install_id"):
        config["install_id"] = str(uuid.uuid4())
        save_config(config)

    relay_url = str(config["relay_url"]).rstrip("/")
    base_payload = {
        "schema_version": load_contract()["schema_version"],
        "limina_version": read_version(),
        "install_id": config["install_id"],
    }
    status, start_data = http_json(f"{relay_url}/api/register/start", payload=base_payload)
    if status != 200 or not start_data:
        return config

    challenge_id = str(start_data.get("challenge_id", ""))
    salt = str(start_data.get("salt", ""))
    difficulty = int(start_data.get("difficulty", 0))
    if not challenge_id or not salt or difficulty <= 0:
        return config

    nonce = solve_pow(challenge_id, salt, difficulty)
    finish_payload = {
        **base_payload,
        "consent_tier": config["consent_state"],
        "challenge_id": challenge_id,
        "nonce": nonce,
    }
    status, finish_data = http_json(f"{relay_url}/api/register/finish", payload=finish_payload)
    if status == 200 and finish_data and finish_data.get("install_token"):
        config["install_token"] = str(finish_data["install_token"])
        save_config(config)
    return config


def ensure_write_token(config: dict[str, Any]) -> str:
    config = register_install(config)
    install_token = str(config.get("install_token") or "")
    if not install_token:
        return ""

    session = load_session()
    token = str(session.get("write_token") or "")
    expires_at = parse_utc(session.get("write_token_expires_at"))
    if token and expires_at and expires_at - utc_now() > WRITE_TOKEN_SAFETY_MARGIN:
        return token

    relay_url = str(config["relay_url"]).rstrip("/")
    headers = {"Authorization": f"Bearer {install_token}"}
    payload = {
        "schema_version": load_contract()["schema_version"],
        "install_id": config["install_id"],
    }
    status, data = http_json(f"{relay_url}/api/session", payload=payload, headers=headers)
    if status != 200 or not data or not data.get("session_token"):
        return ""

    ttl = int(data.get("expires_in_seconds", 900))
    session["write_token"] = str(data["session_token"])
    session["write_token_expires_at"] = utc_iso(utc_now() + timedelta(seconds=max(ttl, 60)))
    save_session(session)
    return session["write_token"]


def flush_outbox(limit: int = 100) -> int:
    config = load_config()
    if not consent_enabled(config):
        return 0

    events = read_outbox()
    if not events:
        return 0

    relay_url = str(config["relay_url"]).rstrip("/")
    write_token = ensure_write_token(config)
    if not write_token:
        return 0

    sent = 0
    while events:
        session_uuid = str(events[0].get("session_uuid", ""))
        batch: list[dict[str, Any]] = []
        for event in events:
            if event.get("session_uuid") != session_uuid or len(batch) >= limit:
                break
            batch.append(event)
        if not batch:
            break

        payload = {
            "schema_version": load_contract()["schema_version"],
            "batch_uuid": str(uuid.uuid4()),
            "seq_start": min(int(item["seq"]) for item in batch),
            "seq_end": max(int(item["seq"]) for item in batch),
            "events": batch,
        }
        headers = {"Authorization": f"Bearer {write_token}"}
        status, data = http_json(f"{relay_url}/api/events", payload=payload, headers=headers)
        if status != 200 or not data or int(data.get("processed", 0)) != len(batch):
            break

        events = events[len(batch):]
        write_outbox(events)
        sent += len(batch)

    return sent


def parse_frontmatter(text: str) -> dict[str, str]:
    lines = text.splitlines()
    if not lines or lines[0].strip() != "---":
        return {}
    metadata: dict[str, str] = {}
    for line in lines[1:]:
        if line.strip() == "---":
            break
        match = FRONTMATTER_LINE_RE.match(line.strip())
        if match:
            metadata[match.group(1)] = match.group(2).strip().strip('"').strip("'")
    return metadata


def parse_blockquote_metadata(text: str) -> dict[str, str]:
    metadata: dict[str, str] = {}
    for line in text.splitlines():
        match = META_LINE_RE.match(line.strip())
        if match:
            metadata[match.group(1)] = match.group(2).strip()
    return metadata


def parse_note_metadata(path: Path) -> dict[str, str]:
    text = path.read_text(encoding="utf-8")
    metadata = parse_blockquote_metadata(text)
    metadata.update(parse_frontmatter(text))
    return metadata


def collect_artifact_ids(directory: Path, prefix: str) -> set[str]:
    ids: set[str] = set()
    if not directory.exists():
        return ids
    for path in directory.glob("*.md"):
        match = ARTIFACT_FILE_RE.match(path.name)
        if match and match.group(1) == prefix:
            ids.add(path.name.split("-", 1)[0])
    return ids


def active_blocker_present(active_path: Path) -> bool:
    if not active_path.exists():
        return False
    lines = active_path.read_text(encoding="utf-8").splitlines()
    capture = False
    content: list[str] = []
    for line in lines:
        stripped = line.strip()
        if stripped == "## Blocker":
            capture = True
            continue
        if capture and stripped.startswith("## "):
            break
        if capture:
            content.append(stripped)
    blocker_text = " ".join(part for part in content if part).strip().lower()
    return blocker_text not in {"", "none", "n/a", "no blocker", "clear"}


def run_command_json(command: list[str], cwd: Path) -> dict[str, Any]:
    env = dict(os.environ)
    env["LIMINA_TELEMETRY_INTERNAL"] = "1"
    completed = subprocess.run(command, cwd=cwd, env=env, capture_output=True, text=True, check=False)
    stdout = completed.stdout.strip()
    if not stdout:
        return {}
    try:
        parsed = json.loads(stdout)
    except json.JSONDecodeError:
        return {}
    return parsed if isinstance(parsed, dict) else {}


def build_snapshot(project_root: Path) -> dict[str, Any]:
    kb_root = project_root / "kb"
    hypothesis_ids = collect_artifact_ids(kb_root / "research/hypotheses", "H")
    experiment_ids = collect_artifact_ids(kb_root / "research/experiments", "E")
    finding_ids = collect_artifact_ids(kb_root / "research/findings", "F")
    literature_ids = collect_artifact_ids(kb_root / "research/literature", "L")
    challenge_review_ids = collect_artifact_ids(kb_root / "reports", "CR")
    strategic_review_ids = collect_artifact_ids(kb_root / "reports", "SR")

    experiment_hypotheses: set[str] = set()
    if (kb_root / "research/experiments").exists():
        for path in (kb_root / "research/experiments").glob("*.md"):
            metadata = parse_note_metadata(path)
            hypothesis = metadata.get("Hypothesis") or metadata.get("hypothesis", "")
            if re.fullmatch(r"H\d{3}", hypothesis):
                experiment_hypotheses.add(hypothesis)

    finding_experiments: set[str] = set()
    if (kb_root / "research/findings").exists():
        for path in (kb_root / "research/findings").glob("*.md"):
            metadata = parse_note_metadata(path)
            experiment = metadata.get("Experiment") or metadata.get("experiment", "")
            if re.fullmatch(r"E\d{3}", experiment):
                finding_experiments.add(experiment)

    validation_json = run_command_json(
        [sys.executable, str(project_root / "scripts/kb_validate.py"), "--kb-root", str(kb_root), "--format", "json"],
        cwd=project_root,
    )
    provenance_json = run_command_json(
        [sys.executable, str(project_root / "scripts/kb_provenance.py"), "--kb-root", str(kb_root), "--format", "json"],
        cwd=project_root,
    )

    return {
        "count_h": len(hypothesis_ids),
        "count_e": len(experiment_ids),
        "count_f": len(finding_ids),
        "count_l": len(literature_ids),
        "count_cr": len(challenge_review_ids),
        "count_sr": len(strategic_review_ids),
        "count_h_without_e": len(hypothesis_ids - experiment_hypotheses),
        "count_e_without_f": len(experiment_ids - finding_experiments),
        "flag_active_blocker_present": active_blocker_present(kb_root / "ACTIVE.md"),
        "flag_validation_clean": bool(validation_json.get("ok", False)),
        "flag_provenance_clean": int(provenance_json.get("count", 0)) == 0,
    }


def parse_property_pairs(pairs: list[str]) -> dict[str, Any]:
    properties: dict[str, Any] = {}
    for pair in pairs:
        if "=" not in pair:
            raise TelemetryError(f"Invalid property assignment: {pair!r}")
        key, value = pair.split("=", 1)
        properties[key.strip()] = value.strip()
    return properties


def print_status(json_output: bool = False) -> int:
    config = load_config()
    session = load_session()
    payload = {
        "consent_state": config["consent_state"],
        "relay_url": config["relay_url"],
        "pending_events": len(read_outbox()),
        "install_registered": bool(config.get("install_token")),
        "active_session": bool(session.get("session_uuid")),
    }
    if json_output:
        print(json.dumps(payload, indent=2, sort_keys=True))
    else:
        print(f"consent_state={payload['consent_state']}")
        print(f"relay_url={payload['relay_url']}")
        print(f"pending_events={payload['pending_events']}")
        print(f"install_registered={str(payload['install_registered']).lower()}")
        print(f"active_session={str(payload['active_session']).lower()}")
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Consent-first telemetry client for Limina.")
    subparsers = parser.add_subparsers(dest="command", required=True)

    consent_parser = subparsers.add_parser("consent", help="Prompt or set telemetry consent.")
    consent_parser.add_argument("--tier", choices=["anonymous", "community", "not_now", "declined"])
    consent_parser.add_argument("--runtime-family", choices=["claude", "codex", "opencode"])
    consent_parser.add_argument("--source", default="manual")

    ensure_parser = subparsers.add_parser("ensure-consent", help="Prompt for telemetry if due.")
    ensure_parser.add_argument("--runtime-family", choices=["claude", "codex", "opencode"])
    ensure_parser.add_argument("--source", default="manual")

    session_parser = subparsers.add_parser("session-open", help="Open a new local session and emit session_started.")
    session_parser.add_argument("--runtime-family", choices=["claude", "codex", "opencode"])
    session_parser.add_argument("--emitter", default="manual")
    session_parser.add_argument("--flush", action="store_true")

    emit_parser = subparsers.add_parser("emit", help="Queue a sanitized telemetry event.")
    emit_parser.add_argument("event_name")
    emit_parser.add_argument("--runtime-family", choices=["claude", "codex", "opencode"])
    emit_parser.add_argument("--emitter", default="manual")
    emit_parser.add_argument("--property", action="append", default=[])
    emit_parser.add_argument("--flush", action="store_true")
    emit_parser.add_argument("--rotate-session", action="store_true")

    flush_parser = subparsers.add_parser("flush", help="Flush queued events to the relay.")
    flush_parser.add_argument("--limit", type=int, default=100)

    snapshot_parser = subparsers.add_parser("snapshot", help="Build and optionally emit a graph snapshot.")
    snapshot_parser.add_argument("--project-root", default=".")
    snapshot_parser.add_argument("--runtime-family", choices=["claude", "codex", "opencode"])
    snapshot_parser.add_argument("--emitter", default="snapshot")
    snapshot_parser.add_argument("--emit", action="store_true")
    snapshot_parser.add_argument("--flush", action="store_true")

    status_parser = subparsers.add_parser("status", help="Show telemetry status.")
    status_parser.add_argument("--json", action="store_true")

    disable_parser = subparsers.add_parser("disable", help="Disable telemetry and clear queued state.")
    disable_parser.add_argument("--source", default="manual")

    return parser.parse_args()


def main() -> int:
    args = parse_args()

    if args.command == "consent":
        choice = args.tier or prompt_for_consent()
        apply_consent_choice(
            choice,
            runtime_family=args.runtime_family,
            emitter=args.source,
            flush=True,
        )
        return 0

    if args.command == "ensure-consent":
        ensure_consent(args.source, runtime_family=args.runtime_family)
        return 0

    if args.command == "session-open":
        runtime_family = detect_runtime_family(args.runtime_family)
        create_local_session(runtime_family)
        emit_event(
            "limina_session_started",
            runtime_family=runtime_family,
            emitter=args.emitter,
            properties={},
            flush=args.flush,
        )
        return 0

    if args.command == "emit":
        runtime_family = detect_runtime_family(args.runtime_family)
        properties = parse_property_pairs(args.property)
        emit_event(
            args.event_name,
            runtime_family=runtime_family,
            emitter=args.emitter,
            properties=properties,
            flush=args.flush,
            rotate_session=args.rotate_session,
        )
        return 0

    if args.command == "flush":
        sent = flush_outbox(limit=args.limit)
        print(sent)
        return 0

    if args.command == "snapshot":
        project_root = Path(args.project_root).resolve()
        snapshot = build_snapshot(project_root)
        if args.emit:
            emit_event(
                "limina_research_graph_snapshot",
                runtime_family=detect_runtime_family(args.runtime_family),
                emitter=args.emitter,
                properties=snapshot,
                flush=args.flush,
            )
        else:
            print(json.dumps(snapshot, indent=2, sort_keys=True))
        return 0

    if args.command == "status":
        return print_status(json_output=args.json)

    if args.command == "disable":
        clear_local_state(load_config(), consent_state="declined")
        return 0

    return 1


if __name__ == "__main__":
    raise SystemExit(main())
