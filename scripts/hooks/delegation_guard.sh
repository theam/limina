#!/bin/bash
# PostToolUse hook: Delegation nudge.
# When the Director writes execution artifacts directly, gently remind about delegation.
# Non-blocking (exit 0).

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

PARSED=$(python3 "$PROJECT_ROOT/scripts/hooks/_parse_hook_input.py" 2>/dev/null) || exit 0
FILE_PATH=$(echo "$PARSED" | head -1)

if [ -z "$FILE_PATH" ]; then
  exit 0
fi

# Check if writing to execution-level directories
if echo "$FILE_PATH" | grep -qE '(^|/)experiments/E[0-9]'; then
  echo "DELEGATION NOTE: You're writing experiment code directly."
  echo "Is this a quick fix, or should this be delegated to the Researcher agent?"
  echo "The Director's strength is taste and judgment — let subagents handle execution."
fi

if echo "$FILE_PATH" | grep -qE '(^|/)src/'; then
  echo "DELEGATION NOTE: You're writing feature code directly."
  echo "Is this a quick fix, or should this be delegated to the Builder agent?"
fi

if echo "$FILE_PATH" | grep -qE 'kb/research/literature/L[0-9]{3}-.*\.md$'; then
  echo "DELEGATION NOTE: You're creating a literature entry directly."
  echo "Consider delegating literature surveys to the Surveyor agent for thoroughness."
fi

exit 0
