#!/bin/bash
# PostToolUse hook: Validate kb/ writes in real-time.
# Non-blocking (exit 0) — outputs errors to stdout for agent context.

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
KB_ROOT="$PROJECT_ROOT/kb"

# Parse hook input
PARSED=$(python3 "$PROJECT_ROOT/scripts/hooks/_parse_hook_input.py" 2>/dev/null) || exit 0
FILE_PATH=$(echo "$PARSED" | head -1)

if [ -z "$FILE_PATH" ]; then
  exit 0
fi

# Only validate writes to kb/ directory — handle both absolute and relative paths
case "$FILE_PATH" in
  */kb/*) ;;          # absolute path containing /kb/
  kb/*) ;;            # relative path starting with kb/
  *) exit 0 ;;        # not a kb/ path
esac

# Skip non-artifact files (INDEX.md, BACKLOG.md, DECISIONS.md, etc.)
if echo "$FILE_PATH" | grep -qE '(INDEX|BACKLOG|DECISIONS|CEO_REQUESTS|CHALLENGE|DASHBOARD)\.md$'; then
  exit 0
fi

# Skip non-md files
if ! echo "$FILE_PATH" | grep -qE '\.md$'; then
  exit 0
fi

# Resolve to absolute path for the validator
if [ ! -f "$FILE_PATH" ]; then
  ABS_PATH="$PROJECT_ROOT/$FILE_PATH"
else
  ABS_PATH="$FILE_PATH"
fi

# Run single-file validation
RESULT=$(python3 "$PROJECT_ROOT/scripts/kb_validate.py" --kb-root "$KB_ROOT" --check-file "$ABS_PATH" --format text 2>&1)
EXIT_CODE=$?

# Only show output if validation actually failed (not on "passed" messages)
if [ $EXIT_CODE -ne 0 ]; then
  echo "KB validation warning for $(basename "$FILE_PATH"):"
  echo "$RESULT"
fi

exit 0
