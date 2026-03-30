#!/bin/bash
# PostToolUse hook: Experiment result gate.
# When an experiment is marked COMPLETED, guide the agent toward explicit decisions.
# Non-blocking (exit 0).

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

PARSED=$(python3 "$PROJECT_ROOT/scripts/hooks/_parse_hook_input.py" 2>/dev/null) || exit 0
FILE_PATH=$(echo "$PARSED" | head -1)

if [ -z "$FILE_PATH" ]; then
  exit 0
fi

# Only fire on experiment files
if ! echo "$FILE_PATH" | grep -qE 'kb/research/experiments/E[0-9]{3}-.*\.md$'; then
  exit 0
fi

# Check if file exists and contains COMPLETED status
if [ ! -f "$FILE_PATH" ]; then
  exit 0
fi

if grep -qiE '(status:\s*COMPLETED|Status\*\*:\s*COMPLETED)' "$FILE_PATH"; then
  EXP_ID=$(basename "$FILE_PATH" | grep -oE 'E[0-9]{3}')

  # Check if Results section has content beyond the template placeholder
  HAS_RESULTS=$(awk '/^## Results/,/^## /' "$FILE_PATH" | grep -v '^##' | grep -v '^$' | grep -v '{' | grep -v '_Reference' | grep -v '_What happened' | head -1)

  if [ -z "$HAS_RESULTS" ]; then
    echo "WARNING: $EXP_ID marked COMPLETED but the Results section appears empty."
    echo "Add actual results data before proceeding."
  fi

  echo ""
  echo "EXPERIMENT GATE — $EXP_ID completed. Before proceeding:"
  echo "  1. Does the data support or reject the hypothesis?"
  echo "  2. Have you written a finding (F{NUM}) linking to this experiment?"
  echo "  3. Spawn the devil's advocate for a mini-review of this result."
  echo "  4. Update the hypothesis status (CONFIRMED or REJECTED)."
fi

exit 0
