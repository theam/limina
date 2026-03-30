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

# Mark session start for checkpoint tracking
SESSION_ID="$(date +%s)-$$"
echo "$SESSION_ID" > "/tmp/limina-session-$(echo "$PROJECT_ROOT" | md5sum 2>/dev/null | cut -c1-8 || echo "$PROJECT_ROOT" | md5 -q | cut -c1-8)"
