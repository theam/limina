#!/bin/bash
# PostToolUse hook: Smart protocol checkpoint.
# Replaces the original 12-line counter with actionable checks.

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PROJECT_HASH=$(echo "$PROJECT_ROOT" | md5sum 2>/dev/null | cut -c1-8 || echo "$PROJECT_ROOT" | md5 -q | cut -c1-8)
COUNTER_FILE="/tmp/limina-checkpoint-${PROJECT_HASH}"
SESSION_FILE="/tmp/limina-session-${PROJECT_HASH}"

# Initialize counter
[ -f "$COUNTER_FILE" ] || echo 0 > "$COUNTER_FILE"

COUNT=$(cat "$COUNTER_FILE")
COUNT=$((COUNT + 1))
echo "$COUNT" > "$COUNTER_FILE"

# Every 25 tool uses: check if BACKLOG.md has been read
if [ $((COUNT % 25)) -eq 0 ]; then
  if [ ! -f "$SESSION_FILE" ] || [ "$(find "$SESSION_FILE" -mmin +30 2>/dev/null)" ]; then
    echo "CHECKPOINT ($COUNT tool calls): You have not re-read BACKLOG.md recently."
    echo "Read kb/mission/BACKLOG.md now to stay oriented on current task state."
  fi
fi

# Every 50 tool uses: run validator + reflection prompt
if [ $((COUNT % 50)) -eq 0 ]; then
  # Run validator quietly — only show errors
  if [ -d "$PROJECT_ROOT/kb" ]; then
    ERRORS=$(python3 "$PROJECT_ROOT/scripts/kb_validate.py" --kb-root "$PROJECT_ROOT/kb" --quiet --format text 2>&1)
    if [ -n "$ERRORS" ]; then
      echo "VALIDATION CHECK ($COUNT tool calls):"
      echo "$ERRORS"
    fi
  fi

  # Reflection prompt
  echo ""
  echo "REFLECTION CHECK ($COUNT tool calls) — Before continuing, answer:"
  echo "  1. What was the original task objective? Are you still pursuing it?"
  echo "  2. What have you assumed in the last 50 steps that you haven't verified?"
  echo "  3. What result would make you reconsider your current approach?"
  echo "If any answer concerns you, spawn /challenge or notify the CEO."
fi
