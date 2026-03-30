#!/usr/bin/env python3
"""Shared JSON parser for Limina hooks. Reads hook input from stdin, outputs shell-safe vars.

For Write: outputs the full content.
For Edit: simulates the edit by applying old_string -> new_string on the existing file.
For MultiEdit: applies all edits sequentially to produce the post-edit content.

Output: line 1 is file_path, line 2+ is the post-edit content.
"""
import json
import sys
from pathlib import Path

try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(0)

# Handle both flat and nested tool_input formats
if "tool_input" in data:
    data = data["tool_input"]

fp = data.get("file_path", "")
content = data.get("content", "")  # Write tool
old_string = data.get("old_string", "")  # Edit tool
new_string = data.get("new_string", "")  # Edit tool
edits = data.get("edits", [])  # MultiEdit tool


def _read_existing(path: str) -> str:
    """Try to read the existing file content."""
    try:
        return Path(path).read_text(encoding="utf-8")
    except Exception:
        return ""


# For Edit operations, simulate the edit to produce the post-edit content
if not content and old_string and fp:
    existing = _read_existing(fp)
    if existing:
        content = existing.replace(old_string, new_string, 1)
    else:
        content = new_string

# For MultiEdit operations, apply all edits sequentially
if not content and edits and fp:
    existing = _read_existing(fp)
    if existing:
        content = existing
        for edit in edits:
            old = edit.get("old_string", "")
            new = edit.get("new_string", "")
            if old:
                content = content.replace(old, new, 1)
    else:
        # File doesn't exist yet — concatenate all new_strings
        content = "\n".join(edit.get("new_string", "") for edit in edits)

# Fallback: if still no content, use new_string directly
if not content:
    content = new_string

# Output: line 1 is file_path, line 2+ is content
print(fp)
print(content)
