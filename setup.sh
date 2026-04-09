#!/bin/bash
# Limina — install the /limina skill for Claude Code and Codex
# Usage: curl -fsSL https://raw.githubusercontent.com/theam/limina/main/setup.sh | bash

set -e

SKILL_URL="https://raw.githubusercontent.com/theam/limina/main/skill/SKILL.md"
installed=0

# Claude Code
if [ -d "$HOME/.claude" ]; then
  mkdir -p "$HOME/.claude/skills/limina"
  curl -fsSL "$SKILL_URL" -o "$HOME/.claude/skills/limina/SKILL.md"
  echo "  ✓ Installed for Claude Code"
  installed=1
fi

# Codex
if [ -d "$HOME/.codex" ]; then
  mkdir -p "$HOME/.codex/skills/limina"
  curl -fsSL "$SKILL_URL" -o "$HOME/.codex/skills/limina/SKILL.md"
  echo "  ✓ Installed for Codex"
  installed=1
fi

# Neither detected — install for Claude Code by default
if [ "$installed" -eq 0 ]; then
  mkdir -p "$HOME/.claude/skills/limina"
  curl -fsSL "$SKILL_URL" -o "$HOME/.claude/skills/limina/SKILL.md"
  echo "  ✓ Installed for Claude Code"
fi

echo ""
echo "  Limina skill installed. Type /limina in Claude Code or Codex to start a research project."
echo "  After you create a project from the template, run 'bash scripts/install_skills.sh' inside that repo to install the bundled companion skills."
echo "  Telemetry stays off by default and will only be offered later inside the project install flow."
