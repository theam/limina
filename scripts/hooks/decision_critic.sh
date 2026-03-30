#!/bin/bash
# PostToolUse hook: Prompt devil's advocate review at decision points.
# Fires on writes to DECISIONS.md and finding files.
# Non-blocking (exit 0).

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

PARSED=$(python3 "$PROJECT_ROOT/scripts/hooks/_parse_hook_input.py" 2>/dev/null) || exit 0
FILE_PATH=$(echo "$PARSED" | head -1)

if [ -z "$FILE_PATH" ]; then
  exit 0
fi

# Trigger on DECISIONS.md writes
if echo "$FILE_PATH" | grep -qE 'kb/mission/DECISIONS\.md$'; then
  echo ""
  echo "DECISION RECORDED. Before finalizing:"
  echo "  - Spawn the devil's advocate to review this decision."
  echo "  - Run /challenge with target 'Engineering decisions' or 'Research direction'."
  echo "  - Ensure the decision references supporting evidence (findings, experiments, data)."
fi

# Trigger on finding file creation
if echo "$FILE_PATH" | grep -qE 'kb/research/findings/F[0-9]{3}-.*\.md$'; then
  FINDING_ID=$(basename "$FILE_PATH" | grep -oE 'F[0-9]{3}')
  echo ""
  echo "FINDING $FINDING_ID RECORDED. Before it informs future decisions:"
  echo "  - The devil's advocate should review this finding."
  echo "  - Does the evidence actually support the conclusion?"
  echo "  - Are there alternative explanations for the observed result?"
fi

exit 0
