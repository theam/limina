from __future__ import annotations

import hashlib
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
CONTRACT_PATH = ROOT / "telemetry/contract.v1.json"
CHECKSUM_PATH = ROOT / "telemetry/contract.v1.sha256"


class ContractChecksumTest(unittest.TestCase):
    def test_checksum_matches(self) -> None:
        expected = CHECKSUM_PATH.read_text(encoding="utf-8").strip()
        actual = hashlib.sha256(CONTRACT_PATH.read_bytes()).hexdigest()
        self.assertEqual(actual, expected)


if __name__ == "__main__":
    unittest.main()
