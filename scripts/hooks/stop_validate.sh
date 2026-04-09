#!/bin/bash
# Stop hook: run a full kb validation before Claude finishes the turn.

exec 0</dev/null

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
KB_ROOT="$PROJECT_ROOT/kb"
TELEMETRY_SCRIPT="$PROJECT_ROOT/scripts/telemetry.py"

if [ ! -d "$KB_ROOT" ]; then
  exit 0
fi

RESULT=$(python3 "$PROJECT_ROOT/scripts/kb_validate.py" --kb-root "$KB_ROOT" --format text 2>&1)
EXIT_CODE=$?

if [ $EXIT_CODE -ne 0 ]; then
  if [ -f "$TELEMETRY_SCRIPT" ]; then
    python3 "$TELEMETRY_SCRIPT" emit limina_session_completed \
      --runtime-family claude \
      --emitter claude_stop \
      --property "result_code=validation_failed" \
      --flush >/dev/null 2>&1 || true
  fi
  echo "BLOCKED: kb validation failed before stop." >&2
  echo "$RESULT" >&2
  exit 2
fi

if [ -f "$TELEMETRY_SCRIPT" ]; then
  python3 "$TELEMETRY_SCRIPT" snapshot \
    --project-root "$PROJECT_ROOT" \
    --runtime-family claude \
    --emitter snapshot \
    --emit >/dev/null 2>&1 || true
  python3 "$TELEMETRY_SCRIPT" emit limina_session_completed \
    --runtime-family claude \
    --emitter claude_stop \
    --property "result_code=success" \
    --flush >/dev/null 2>&1 || true
fi

exit 0
