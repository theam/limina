#!/usr/bin/env python3
"""Print the next Limina artifact ID derived from the filesystem."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


ARTIFACT_DIRS = {
    "H": Path("kb/research/hypotheses"),
    "E": Path("kb/research/experiments"),
    "F": Path("kb/research/findings"),
    "L": Path("kb/research/literature"),
    "CR": Path("kb/reports"),
    "SR": Path("kb/reports"),
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Derive the next Limina artifact ID from the filesystem.")
    parser.add_argument("prefix", choices=sorted(ARTIFACT_DIRS), help="Artifact prefix to allocate")
    parser.add_argument(
        "--project-root",
        default=".",
        help="Project root containing kb/ (default: current directory)",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    prefix = args.prefix
    project_root = Path(args.project_root).resolve()
    target_dir = project_root / ARTIFACT_DIRS[prefix]
    pattern = re.compile(rf"^{prefix}(\d{{3}})-.+\.md$")

    highest = 0
    if target_dir.exists():
        for path in target_dir.glob("*.md"):
            match = pattern.match(path.name)
            if match:
                highest = max(highest, int(match.group(1)))

    print(f"{prefix}{highest + 1:03d}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
