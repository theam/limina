#!/usr/bin/env bash
# Hook: PostToolUse — reminds the agent to run /close-mission when all tasks are DONE.
# Runs every 10 tool uses to avoid excessive overhead.

set -euo pipefail

TOOL_USE_COUNT="${LIMINA_TOOL_USE_COUNT:-0}"

# Only check every 10 tool uses
if (( TOOL_USE_COUNT % 10 != 0 )) || (( TOOL_USE_COUNT == 0 )); then
    exit 0
fi

BACKLOG="kb/mission/BACKLOG.md"

if [[ ! -f "$BACKLOG" ]]; then
    exit 0
fi

# Check if there are any tasks and if all are DONE
TOTAL_TASKS=$(grep -cE '^\| T[0-9]{3}' "$BACKLOG" 2>/dev/null || echo "0")
DONE_TASKS=$(grep -cE '^\| T[0-9]{3}.*\| DONE' "$BACKLOG" 2>/dev/null || echo "0")

if (( TOTAL_TASKS > 0 )) && (( TOTAL_TASKS == DONE_TASKS )); then
    # Check if /close-mission was already run (shared-knowledge has cards or kb/cards exists)
    CARDS_EXIST=false
    if ls shared-knowledge/cards/K*.md 1>/dev/null 2>&1; then
        CARDS_EXIST=true
    fi
    if ls kb/cards/K*.md 1>/dev/null 2>&1; then
        CARDS_EXIST=true
    fi

    if [[ "$CARDS_EXIST" == "false" ]]; then
        echo "⚠️  All $TOTAL_TASKS tasks are DONE but no Knowledge Cards have been generated."
        echo "   Run /close-mission to synthesize reusable knowledge before ending this mission."
    fi
fi
