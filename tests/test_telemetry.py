from __future__ import annotations

import importlib
import json
import os
import sys
import tempfile
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS_DIR = ROOT / "scripts"
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import telemetry  # type: ignore  # noqa: E402


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


if __name__ == "__main__":
    unittest.main()
