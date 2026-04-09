from __future__ import annotations

import argparse
import contextlib
import io
import importlib
import json
import os
import sys
import tempfile
from pathlib import Path
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS_DIR = ROOT / "scripts"
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import telemetry  # type: ignore  # noqa: E402
import kb_provenance  # type: ignore  # noqa: E402
import kb_validate  # type: ignore  # noqa: E402


class TelemetryClientTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tempdir = tempfile.TemporaryDirectory()
        os.environ["LIMINA_HOME"] = self.tempdir.name
        importlib.reload(telemetry)

    def tearDown(self) -> None:
        self.tempdir.cleanup()
        os.environ.pop("LIMINA_HOME", None)
        importlib.reload(telemetry)

    def enable_telemetry(self) -> None:
        config = telemetry.load_config()
        config["consent_state"] = "anonymous"
        telemetry.save_config(config)

    def test_emit_event_writes_sanitized_outbox(self) -> None:
        self.enable_telemetry()

        queued = telemetry.emit_event(
            "limina_artifact_created",
            runtime_family="codex",
            emitter="kb_new_artifact",
            properties={"artifact_type": "H"},
        )

        self.assertTrue(queued)
        events = telemetry.read_outbox()
        self.assertEqual(len(events), 1)
        self.assertEqual(events[0]["event"], "limina_artifact_created")
        self.assertEqual(events[0]["properties"]["artifact_type"], "H")
        self.assertNotIn("file_path", events[0]["properties"])

    def test_unknown_property_is_rejected(self) -> None:
        self.enable_telemetry()

        with self.assertRaises(telemetry.TelemetryError):
            telemetry.emit_event(
                "limina_artifact_created",
                runtime_family="codex",
                emitter="kb_new_artifact",
                properties={"artifact_type": "H", "file_path": "/tmp/private.txt"},
            )

    def test_not_now_suppresses_prompt_without_queueing(self) -> None:
        telemetry.apply_consent_choice("not_now", emitter="manual")
        config = telemetry.load_config()

        self.assertEqual(config["consent_state"], "unset")
        self.assertEqual(telemetry.read_outbox(), [])
        self.assertTrue(config["prompt_suppressed_until"])

    def test_session_completed_clears_local_session(self) -> None:
        self.enable_telemetry()
        telemetry.emit_event(
            "limina_session_started",
            runtime_family="claude",
            emitter="claude_session_start",
            properties={},
        )

        self.assertTrue(Path(telemetry.SESSION_PATH).exists())

        telemetry.emit_event(
            "limina_session_completed",
            runtime_family="claude",
            emitter="claude_stop",
            properties={"result_code": "success"},
        )

        self.assertFalse(Path(telemetry.SESSION_PATH).exists())
        events = telemetry.read_outbox()
        self.assertEqual(events[-1]["event"], "limina_session_completed")

    def test_disable_clears_local_state(self) -> None:
        self.enable_telemetry()
        telemetry.emit_event(
            "limina_artifact_created",
            runtime_family="codex",
            emitter="kb_new_artifact",
            properties={"artifact_type": "H"},
        )

        telemetry.clear_local_state(telemetry.load_config(), consent_state="declined")

        self.assertEqual(telemetry.load_config()["consent_state"], "declined")
        self.assertFalse(telemetry.read_outbox())
        self.assertFalse(Path(telemetry.SESSION_PATH).exists())

    def test_ensure_write_token_reregisters_after_install_auth_failure(self) -> None:
        self.enable_telemetry()
        config = telemetry.load_config()
        config["install_id"] = "install-123"
        config["install_token"] = "stale-token"
        telemetry.save_config(config)
        relay_url = str(config["relay_url"]).rstrip("/")

        calls: list[tuple[str, str]] = []

        def fake_http_json(
            url: str,
            *,
            method: str = "POST",
            payload: dict[str, object] | None = None,
            headers: dict[str, str] | None = None,
            timeout: int = 10,
        ) -> tuple[int | None, dict[str, object] | None]:
            del method, payload, timeout
            auth = headers.get("Authorization", "") if headers else ""
            calls.append((url, auth))
            if url.endswith("/api/session") and auth == "Bearer stale-token":
                return 401, {}
            if url.endswith("/api/register/start"):
                return 200, {"challenge_id": "challenge-1", "salt": "salt-1", "difficulty": 1}
            if url.endswith("/api/register/finish"):
                return 200, {"install_token": "fresh-token"}
            if url.endswith("/api/session") and auth == "Bearer fresh-token":
                return 200, {"session_token": "write-token", "expires_in_seconds": 900}
            self.fail(f"Unexpected telemetry request: url={url!r} auth={auth!r}")

        with (
            mock.patch.object(telemetry, "http_json", side_effect=fake_http_json),
            mock.patch.object(telemetry, "solve_pow", return_value=7),
        ):
            token = telemetry.ensure_write_token(telemetry.load_config())

        self.assertEqual(token, "write-token")
        self.assertEqual(telemetry.load_config()["install_token"], "fresh-token")
        self.assertEqual(
            calls,
            [
                (f"{relay_url}/api/session", "Bearer stale-token"),
                (f"{relay_url}/api/register/start", ""),
                (f"{relay_url}/api/register/finish", ""),
                (f"{relay_url}/api/session", "Bearer fresh-token"),
            ],
        )


class TelemetryPromptSafetyTest(unittest.TestCase):
    def make_valid_kb(self, root: Path) -> Path:
        kb_root = root / "kb"
        (kb_root / "mission").mkdir(parents=True, exist_ok=True)
        (kb_root / "ACTIVE.md").write_text(
            """---
aliases: ["ACTIVE"]
type: active-state
---

# Active State

## Current Objective

Validate telemetry prompt behavior.

## Next Step

Keep machine-readable output clean.

## Blocker

None.

## Links

- Mission: [[CHALLENGE]]
""",
            encoding="utf-8",
        )
        (kb_root / "mission" / "CHALLENGE.md").write_text(
            """---
aliases: ["CHALLENGE"]
type: mission
---

# Research Mission

## Objective

Test telemetry prompt gating.

## Context

Minimal fixture.

## Success Criteria

Structured outputs remain machine-readable.

## Constraints

- Keep telemetry opt-in.

## Links

- Active State: [[ACTIVE]]
- Dashboard: [[DASHBOARD]]
""",
            encoding="utf-8",
        )
        (kb_root / "DASHBOARD.md").write_text(
            """---
aliases: ["DASHBOARD"]
type: dashboard
---

# Dashboard

## Entry Points

- Mission: [[CHALLENGE]]
- Active State: [[ACTIVE]]

## Links

- Mission: [[CHALLENGE]]
- Active State: [[ACTIVE]]
""",
            encoding="utf-8",
        )
        return kb_root

    def test_kb_validate_json_mode_skips_prompt_and_emits_clean_json(self) -> None:
        with tempfile.TemporaryDirectory() as tempdir:
            kb_root = self.make_valid_kb(Path(tempdir))
            consent_mock = mock.Mock()
            args = argparse.Namespace(kb_root=str(kb_root), check_file=None, format="json", quiet=False)
            stdout = io.StringIO()
            with (
                mock.patch.object(kb_validate, "parse_args", return_value=args),
                mock.patch.object(kb_validate, "telemetry_ensure_consent", consent_mock),
                contextlib.redirect_stdout(stdout),
            ):
                exit_code = kb_validate.main()

            self.assertEqual(exit_code, 0)
            payload = json.loads(stdout.getvalue())
            self.assertTrue(payload["ok"])
            consent_mock.assert_not_called()

    def test_kb_validate_check_file_mode_skips_prompt(self) -> None:
        with tempfile.TemporaryDirectory() as tempdir:
            kb_root = self.make_valid_kb(Path(tempdir))
            consent_mock = mock.Mock()
            args = argparse.Namespace(
                kb_root=str(kb_root),
                check_file=str(kb_root / "ACTIVE.md"),
                format="text",
                quiet=True,
            )
            stdout = io.StringIO()
            with (
                mock.patch.object(kb_validate, "parse_args", return_value=args),
                mock.patch.object(kb_validate, "telemetry_ensure_consent", consent_mock),
                contextlib.redirect_stdout(stdout),
            ):
                exit_code = kb_validate.main()

            self.assertEqual(exit_code, 0)
            consent_mock.assert_not_called()

    def test_kb_provenance_json_mode_skips_prompt_and_emits_clean_json(self) -> None:
        with tempfile.TemporaryDirectory() as tempdir:
            kb_root = self.make_valid_kb(Path(tempdir))
            consent_mock = mock.Mock()
            args = argparse.Namespace(kb_root=str(kb_root), max_age_days=180, format="json")
            stdout = io.StringIO()
            with (
                mock.patch.object(kb_provenance, "parse_args", return_value=args),
                mock.patch.object(kb_provenance, "telemetry_ensure_consent", consent_mock),
                contextlib.redirect_stdout(stdout),
            ):
                exit_code = kb_provenance.main()

            self.assertEqual(exit_code, 0)
            payload = json.loads(stdout.getvalue())
            self.assertEqual(payload["count"], 0)
            consent_mock.assert_not_called()


if __name__ == "__main__":
    unittest.main()
