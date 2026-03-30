#!/bin/bash
# SessionStart hook: Inject full operating context at session start.
# Claude Code injects stdout into agent context automatically.

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

# --- CLAUDE.md ---
if [ -f "$PROJECT_ROOT/CLAUDE.md" ]; then
  echo "=== CLAUDE.md ==="
  cat "$PROJECT_ROOT/CLAUDE.md"
  echo ""
else
  echo "=== WARNING: CLAUDE.md not found ==="
fi

# --- kb/INDEX.md ---
if [ -f "$PROJECT_ROOT/kb/INDEX.md" ]; then
  echo "=== kb/INDEX.md ==="
  cat "$PROJECT_ROOT/kb/INDEX.md"
  echo ""
else
  echo "=== WARNING: kb/INDEX.md not found (fresh project?) ==="
fi

# --- kb/mission/BACKLOG.md ---
if [ -f "$PROJECT_ROOT/kb/mission/BACKLOG.md" ]; then
  echo "=== kb/mission/BACKLOG.md ==="
  cat "$PROJECT_ROOT/kb/mission/BACKLOG.md"
  echo ""
else
  echo "=== WARNING: kb/mission/BACKLOG.md not found (fresh project?) ==="
fi

# Mark session start for checkpoint tracking — use same hash logic as protocol_checkpoint.sh
if command -v md5sum >/dev/null 2>&1; then
  PROJECT_HASH=$(echo "$PROJECT_ROOT" | md5sum | cut -c1-8)
elif command -v md5 >/dev/null 2>&1; then
  PROJECT_HASH=$(echo "$PROJECT_ROOT" | md5 -q | cut -c1-8)
else
  PROJECT_HASH=$(echo "$PROJECT_ROOT" | cksum | cut -d' ' -f1)
fi
SESSION_ID="$(date +%s)-$$"
echo "$SESSION_ID" > "/tmp/limina-session-${PROJECT_HASH}"
