---
name: notion-sync-kb
description: Sync the repository knowledge base (kb/) to Notion using scripts/notion_sync.py. Use when asked to export, mirror, or update Notion from kb markdown files, or to run dry-run/force sync checks.
---

# Notion Sync KB

Run one-way sync from local `kb/` to Notion.

## Preconditions

Required environment variables:

- `NOTION_API_KEY`
- `NOTION_ROOT_PAGE_ID`

Quick check:

```bash
echo "API_KEY: ${NOTION_API_KEY:+SET}" && echo "ROOT_PAGE: ${NOTION_ROOT_PAGE_ID:+SET}"
```

## Sync Modes

- Dry run: `uv run --with notion-client --with python-frontmatter python3 scripts/notion_sync.py --dry-run`
- Normal sync: `uv run --with notion-client --with python-frontmatter python3 scripts/notion_sync.py`
- Force sync: `uv run --with notion-client --with python-frontmatter python3 scripts/notion_sync.py --force`

Ask the user which mode to run when it is not specified.

## Reporting

After execution, report:

- created / updated / skipped counts
- any errors
- root Notion page identifier used

## Notes

- Script entrypoint: `scripts/notion_sync.py`
- Sync state cache: `kb/.notion-sync.json`
- Databases synced: hypotheses, experiments, findings, and literature
- Root page content synced from: `kb/ACTIVE.md`
- Mission and report files sync as pages under their sections
