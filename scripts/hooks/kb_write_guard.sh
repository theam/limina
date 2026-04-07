#!/bin/bash
# PostToolUse hook: validate kb/ writes immediately.
# Exit code 2 surfaces the error back to Claude Code so invalid kb edits do not go unnoticed.

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
KB_ROOT="$PROJECT_ROOT/kb"

PARSED=$(python3 "$PROJECT_ROOT/scripts/hooks/_parse_hook_input.py" 2>/dev/null) || exit 0
FILE_PATH=$(echo "$PARSED" | head -1)

if [ -z "$FILE_PATH" ]; then
  exit 0
fi

case "$FILE_PATH" in
  */kb/*|kb/*) ;;
  *) exit 0 ;;
esac

if ! echo "$FILE_PATH" | grep -qE '\.md$'; then
  exit 0
fi

if echo "$FILE_PATH" | grep -qE '(^|/)kb/(research/data|lessons)/'; then
  exit 0
fi

if echo "$FILE_PATH" | grep -qE '/\.'; then
  exit 0
fi

if [ ! -f "$FILE_PATH" ]; then
  ABS_PATH="$PROJECT_ROOT/$FILE_PATH"
else
  ABS_PATH="$FILE_PATH"
fi

RESULT=$(python3 "$PROJECT_ROOT/scripts/kb_validate.py" --kb-root "$KB_ROOT" --check-file "$ABS_PATH" --format text 2>&1)
EXIT_CODE=$?

if [ $EXIT_CODE -ne 0 ]; then
  echo "BLOCKED: KB validation failed for $(basename "$FILE_PATH")." >&2
  echo "$RESULT" >&2
  exit 2
fi

exit 0
