#!/usr/bin/env python3
"""Shared JSON parser for Limina hooks. Reads hook input from stdin, outputs shell-safe vars.

For Write: outputs the full content.
For Edit: simulates the edit by applying old_string → new_string on the existing file,
          so downstream hooks validate the post-edit state, not the pre-edit file.
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

# For Edit operations, simulate the edit to produce the post-edit content
if not content and old_string and fp:
    try:
        existing = Path(fp).read_text(encoding="utf-8")
        content = existing.replace(old_string, new_string, 1)
    except Exception:
        # File may not exist yet or path may be relative
        content = new_string

# Fallback: if still no content, use new_string directly
if not content:
    content = new_string

# Output: line 1 is file_path, line 2+ is content
print(fp)
print(content)
