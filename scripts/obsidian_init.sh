#!/bin/bash
# Initialize Obsidian vault configuration for the Limina knowledge base.
# Run: bash scripts/obsidian_init.sh
# This is optional — the kb/ remains a Git-backed Markdown source of truth.

set -e

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
KB_ROOT="$PROJECT_ROOT/kb"
OBSIDIAN_DIR="$KB_ROOT/.obsidian"

echo "Initializing Obsidian vault for Limina knowledge base..."

# Create .obsidian config directory
mkdir -p "$OBSIDIAN_DIR"

# Basic Obsidian app settings
cat > "$OBSIDIAN_DIR/app.json" << 'EOF'
{
  "showLineNumber": true,
  "strictLineBreaks": true,
  "readableLineLength": true,
  "showFrontmatter": true,
  "defaultViewMode": "source"
}
EOF

# Community plugins list (Dataview is required for dashboard queries)
cat > "$OBSIDIAN_DIR/community-plugins.json" << 'EOF'
[
  "dataview"
]
EOF

# Graph view color groups
cat > "$OBSIDIAN_DIR/graph.json" << 'EOF'
{
  "colorGroups": [
    {
      "query": "path:research",
      "color": { "a": 1, "rgb": 4478207 }
    },
    {
      "query": "path:engineering",
      "color": { "a": 1, "rgb": 3394611 }
    },
    {
      "query": "path:reports",
      "color": { "a": 1, "rgb": 16734003 }
    },
    {
      "query": "path:tasks",
      "color": { "a": 1, "rgb": 16776960 }
    },
    {
      "query": "path:mission",
      "color": { "a": 1, "rgb": 16777215 }
    }
  ]
}
EOF

# Create DASHBOARD.md with Dataview queries
cat > "$KB_ROOT/DASHBOARD.md" << 'DASHEOF'
---
type: dashboard
---

# Limina Research Dashboard

> This dashboard uses [Dataview](https://github.com/blacksmithgu/obsidian-dataview) queries. Install the Dataview community plugin to see live tables.

## Active Tasks

```dataview
TABLE status, priority, task_type as "Type"
FROM "tasks"
WHERE status != "DONE"
SORT priority ASC
```

## Recent Experiments

```dataview
TABLE status, hypothesis, task
FROM "research/experiments"
SORT created DESC
LIMIT 10
```

## Hypotheses

```dataview
TABLE status, task
FROM "research/hypotheses"
SORT created DESC
```

## Findings

```dataview
TABLE impact, hypothesis, experiment
FROM "research/findings"
SORT created DESC
```

## Literature

```dataview
TABLE source_type as "Type", relevance, task
FROM "research/literature"
SORT created DESC
LIMIT 15
```

## Engineering Features

```dataview
TABLE status, task
FROM "engineering/features"
SORT created DESC
```

## Challenge Reviews

```dataview
TABLE target, requested_by as "Requested By"
FROM "reports"
WHERE type = "challenge-review"
SORT created DESC
LIMIT 5
```

## Decisions

See [DECISIONS.md](mission/DECISIONS.md) for the full decision log.
DASHEOF

# Add Obsidian user-specific files to .gitignore if not already there
GITIGNORE="$PROJECT_ROOT/.gitignore"
if [ -f "$GITIGNORE" ]; then
  if ! grep -q '.obsidian/workspace' "$GITIGNORE" 2>/dev/null; then
    echo "" >> "$GITIGNORE"
    echo "# Obsidian user-specific files" >> "$GITIGNORE"
    echo "kb/.obsidian/workspace.json" >> "$GITIGNORE"
    echo "kb/.obsidian/workspace-mobile.json" >> "$GITIGNORE"
    echo "kb/.obsidian/hotkeys.json" >> "$GITIGNORE"
    echo "kb/.obsidian/appearance.json" >> "$GITIGNORE"
  fi
else
  cat > "$GITIGNORE" << 'EOF'
# Obsidian user-specific files
kb/.obsidian/workspace.json
kb/.obsidian/workspace-mobile.json
kb/.obsidian/hotkeys.json
kb/.obsidian/appearance.json
EOF
fi

echo ""
echo "Obsidian vault initialized at: $KB_ROOT"
echo ""
echo "To use:"
echo "  1. Open Obsidian"
echo "  2. Click 'Open folder as vault'"
echo "  3. Select: $KB_ROOT"
echo "  4. Install the Dataview community plugin (Settings → Community plugins → Browse → 'Dataview')"
echo "  5. Open DASHBOARD.md for a live overview of your research"
echo ""
echo "Note: The kb/ remains a Git-backed Markdown source of truth."
echo "Obsidian is the human UI layer — all data lives in plain Markdown + YAML frontmatter."
