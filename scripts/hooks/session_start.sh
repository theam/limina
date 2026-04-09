#!/bin/bash
# SessionStart hook: inject only the small runtime state the agent needs now.

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TELEMETRY_SCRIPT="$PROJECT_ROOT/scripts/telemetry.py"

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

if [ -f "$TELEMETRY_SCRIPT" ]; then
  python3 "$TELEMETRY_SCRIPT" ensure-consent --runtime-family claude --source claude_session_start || true
  python3 "$TELEMETRY_SCRIPT" session-open --runtime-family claude --emitter claude_session_start --flush >/dev/null 2>&1 || true
fi
