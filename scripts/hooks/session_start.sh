#!/bin/bash
# SessionStart hook: inject only the small runtime state the agent needs now.

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

emit_file() {
  local label="$1"
  local path="$2"
  if [ -f "$path" ]; then
    echo "=== $label ==="
    cat "$path"
    echo ""
  else
    echo "=== WARNING: $label not found ==="
  fi
}

emit_file "kb/mission/CHALLENGE.md" "$PROJECT_ROOT/kb/mission/CHALLENGE.md"
emit_file "kb/ACTIVE.md" "$PROJECT_ROOT/kb/ACTIVE.md"
