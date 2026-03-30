#!/usr/bin/env python3
"""Shared JSON parser for Limina hooks. Reads hook input from stdin, outputs shell-safe vars."""
import json
import sys

try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(0)

# Handle both flat and nested tool_input formats
if "tool_input" in data:
    data = data["tool_input"]

fp = data.get("file_path", "")
content = data.get("content", "") or data.get("new_string", "")

# Output as simple key=value lines (no quoting issues — consumers read specific lines)
print(fp)       # line 1: file_path
print(content)  # line 2: content (may be multiline, but line 1 is always the path)
