#!/bin/bash
# Initialize a lightweight Obsidian vault for the slim Limina kb/.

set -e

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
KB_ROOT="$PROJECT_ROOT/kb"
OBSIDIAN_DIR="$KB_ROOT/.obsidian"

echo "Initializing Obsidian vault for Limina..."

mkdir -p "$OBSIDIAN_DIR"

cat > "$OBSIDIAN_DIR/app.json" << 'EOF'
{
  "showLineNumber": true,
  "strictLineBreaks": true,
  "readableLineLength": true,
  "showFrontmatter": true,
  "defaultViewMode": "source"
}
EOF

cat > "$OBSIDIAN_DIR/community-plugins.json" << 'EOF'
[
  "dataview"
]
EOF

cat > "$OBSIDIAN_DIR/graph.json" << 'EOF'
{
  "colorGroups": [
    {
      "query": "path:research",
      "color": { "a": 1, "rgb": 4478207 }
    },
    {
      "query": "path:reports",
      "color": { "a": 1, "rgb": 16734003 }
    },
    {
      "query": "path:lessons",
      "color": { "a": 1, "rgb": 16776960 }
    },
    {
      "query": "path:mission",
      "color": { "a": 1, "rgb": 16777215 }
    }
  ]
}
EOF

cat > "$KB_ROOT/DASHBOARD.md" << 'EOF'
---
aliases: ["DASHBOARD"]
type: dashboard
---

# Limina Dashboard

## Entry Points

- [[ACTIVE]]
- [[CHALLENGE]]

## Links

- Active State: [[ACTIVE]]
- Mission: [[CHALLENGE]]

## Active Working Set

```dataview
TABLE file.outlinks as "Outlinks"
FROM ""
WHERE file.name = "ACTIVE"
```

## Core Graph

```dataview
TABLE file.folder as "Folder", length(file.inlinks) as "In", length(file.outlinks) as "Out"
FROM ""
WHERE startswith(file.path, "research/") OR startswith(file.path, "reports/") OR startswith(file.path, "lessons/")
SORT file.name ASC
```

## Orphan Notes

```dataview
TABLE file.folder as "Folder"
FROM ""
WHERE file.name != "README"
AND file.name != "DASHBOARD"
AND length(file.inlinks) = 0
AND length(file.outlinks) = 0
SORT file.path ASC
```

## Notes Missing Outlinks

```dataview
TABLE file.folder as "Folder", length(file.inlinks) as "In"
FROM ""
WHERE file.name != "README"
AND file.name != "DASHBOARD"
AND length(file.outlinks) = 0
SORT file.path ASC
```

## Notes Missing Inlinks

```dataview
TABLE file.folder as "Folder", length(file.outlinks) as "Out"
FROM ""
WHERE file.name != "README"
AND file.name != "DASHBOARD"
AND length(file.inlinks) = 0
SORT file.path ASC
```
EOF

GITIGNORE="$PROJECT_ROOT/.gitignore"
if [ -f "$GITIGNORE" ]; then
  if ! grep -q 'kb/.obsidian/workspace.json' "$GITIGNORE" 2>/dev/null; then
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
echo "Open the folder as a vault and install Dataview to use DASHBOARD.md."
