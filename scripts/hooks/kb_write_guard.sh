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

# Only validate writes to kb/ directory
if ! echo "$FILE_PATH" | grep -q '/kb/'; then
  exit 0
fi

# Skip non-artifact files (INDEX.md, BACKLOG.md, DECISIONS.md, etc.)
if echo "$FILE_PATH" | grep -qE '(INDEX|BACKLOG|DECISIONS|CEO_REQUESTS|CHALLENGE|DASHBOARD)\.md$'; then
  exit 0
fi

# Skip non-md files
if ! echo "$FILE_PATH" | grep -qE '\.md$'; then
  exit 0
fi

# Run single-file validation
RESULT=$(python3 "$PROJECT_ROOT/scripts/kb_validate.py" --kb-root "$KB_ROOT" --check-file "$FILE_PATH" --format text 2>&1)

if echo "$RESULT" | grep -qi "error\|missing\|failed"; then
  echo "KB validation warning for $(basename "$FILE_PATH"):"
  echo "$RESULT"
fi

exit 0
