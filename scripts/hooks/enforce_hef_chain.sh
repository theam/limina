#!/bin/bash
# PreToolUse hook: Enforce H→E→F chain.
# Blocks experiment creation without a hypothesis, and finding creation without an experiment.
# Exit code 2 = blocking error (stderr fed back to Claude as error message).

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
KB_ROOT="$PROJECT_ROOT/kb"

# Parse hook input — line 1 is file_path, line 2+ is content
PARSED=$(python3 "$PROJECT_ROOT/scripts/hooks/_parse_hook_input.py" 2>/dev/null) || exit 0
FILE_PATH=$(echo "$PARSED" | head -1)
CONTENT=$(echo "$PARSED" | tail -n +2)

if [ -z "$FILE_PATH" ]; then
  exit 0
fi

# Extract a metadata reference from content using Python — only from
# YAML frontmatter or blockquote metadata lines, NOT from body text.
extract_meta_ref() {
  local content="$1"
  local field="$2"      # e.g. "hypothesis" or "Hypothesis"
  local ref_pattern="$3" # e.g. "H[0-9]{3}"
  local file="$4"       # existing file to check (may not exist)

  python3 -c "
import re, sys
try:
    import frontmatter
    has_fm = True
except ImportError:
    has_fm = False

content = sys.stdin.read()
field_lower = '${field}'.lower()
ref_re = re.compile(r'(${ref_pattern})')
meta_re = re.compile(r'^>\s+\*\*(.+?)\*\*:\s*(.+?)\s*$')

# Try YAML frontmatter
if has_fm:
    try:
        post = frontmatter.loads(content)
        val = str(post.metadata.get(field_lower, ''))
        m = ref_re.search(val)
        if m:
            print(m.group(1))
            sys.exit(0)
    except Exception:
        pass

# Try blockquote metadata
for line in content.splitlines():
    mm = meta_re.match(line.strip())
    if mm and mm.group(1).strip().lower() == field_lower:
        m = ref_re.search(mm.group(2))
        if m:
            print(m.group(1))
            sys.exit(0)

# Nothing found in content
" <<< "$content"
}

# Check if this is an experiment file being created/edited
if echo "$FILE_PATH" | grep -qE 'kb/research/experiments/E[0-9]{3}-.*\.md$'; then
  # _parse_hook_input.py already simulates Edit results, so CONTENT
  # reflects the post-edit state for both Write and Edit operations.
  HYPO_ID=$(extract_meta_ref "$CONTENT" "hypothesis" 'H[0-9]{3}' "$FILE_PATH")

  if [ -z "$HYPO_ID" ]; then
    echo "BLOCKED: Cannot create experiment without a hypothesis reference." >&2
    echo "Add 'hypothesis: \"H{NUM}\"' to the YAML frontmatter or '> **Hypothesis**: H{NUM}' to the metadata." >&2
    exit 2
  fi

  # Check that the hypothesis file exists
  HYPO_FILE=$(find "$KB_ROOT/research/hypotheses/" -name "${HYPO_ID}-*.md" 2>/dev/null | head -1)
  if [ -z "$HYPO_FILE" ]; then
    echo "BLOCKED: Experiment references $HYPO_ID, but no hypothesis file found in kb/research/hypotheses/." >&2
    echo "Create the hypothesis file first (Rule 3: NEVER run an experiment without a hypothesis file)." >&2
    exit 2
  fi
fi

# Check if this is a finding file being created/edited
if echo "$FILE_PATH" | grep -qE 'kb/research/findings/F[0-9]{3}-.*\.md$'; then
  EXP_ID=$(extract_meta_ref "$CONTENT" "experiment" 'E[0-9]{3}' "$FILE_PATH")

  if [ -z "$EXP_ID" ]; then
    echo "BLOCKED: Cannot create finding without an experiment reference." >&2
    echo "Add 'experiment: \"E{NUM}\"' to the YAML frontmatter or '> **Experiment**: E{NUM}' to the metadata." >&2
    exit 2
  fi

  EXP_FILE=$(find "$KB_ROOT/research/experiments/" -name "${EXP_ID}-*.md" 2>/dev/null | head -1)
  if [ -z "$EXP_FILE" ]; then
    echo "BLOCKED: Finding references $EXP_ID, but no experiment file found in kb/research/experiments/." >&2
    echo "Create the experiment file first (Rule 4: NEVER create a finding without linking it to an experiment)." >&2
    exit 2
  fi
fi

exit 0
