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

Check quickly:

```bash
echo "API_KEY: ${NOTION_API_KEY:+SET}" && echo "ROOT_PAGE: ${NOTION_ROOT_PAGE_ID:+SET}"
```

If missing, ask the user for the values and export them before running sync.

## Sync Modes

- Dry run (recommended first): `uv run --with notion-client --with python-frontmatter python3 scripts/notion_sync.py --dry-run`
- Normal sync: `uv run --with notion-client --with python-frontmatter python3 scripts/notion_sync.py`
- Force sync: `uv run --with notion-client --with python-frontmatter python3 scripts/notion_sync.py --force`

Ask the user which mode to run when it is not specified.

## Reporting

After execution, report:

- Created / updated / skipped counts
- Any errors
- Root Notion page URL or identifier used

## Notes

- Script entrypoint: `scripts/notion_sync.py`
- Sync state cache: `kb/.notion-sync.json`
- Databases synced: hypotheses, experiments, findings, literature, features, investigations, implementations, retrospectives, and tasks
- Mission pages synced: CHALLENGE.md, BACKLOG.md, DECISIONS.md, CEO_REQUESTS.md
