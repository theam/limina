#!/bin/bash
# Stop hook: run a full kb validation before Claude finishes the turn.

exec 0</dev/null

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
KB_ROOT="$PROJECT_ROOT/kb"

if [ ! -d "$KB_ROOT" ]; then
  exit 0
fi

RESULT=$(python3 "$PROJECT_ROOT/scripts/kb_validate.py" --kb-root "$KB_ROOT" --format text 2>&1)
EXIT_CODE=$?

if [ $EXIT_CODE -ne 0 ]; then
  echo "BLOCKED: kb validation failed before stop." >&2
  echo "$RESULT" >&2
  exit 2
fi

exit 0
