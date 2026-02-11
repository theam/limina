# Notion Sync — Export knowledge base to Notion

You are syncing the project's knowledge base (`kb/`) to Notion. This is a one-way sync: kb/ → Notion.

## Prerequisites

The sync requires two environment variables:
- `NOTION_API_KEY` — Notion integration token
- `NOTION_ROOT_PAGE_ID` — ID of the root page in Notion where the kb/ will be mirrored

## Process

### Step 1: Check credentials

Check if both environment variables are set by running:

```bash
echo "API_KEY: ${NOTION_API_KEY:+SET}" && echo "ROOT_PAGE: ${NOTION_ROOT_PAGE_ID:+SET}"
```

If either is missing, use `AskUserQuestion` to request them from the CEO:

- **NOTION_API_KEY**: "I need a Notion integration token. Create one at https://www.notion.so/my-integrations, then share the target page with the integration."
- **NOTION_ROOT_PAGE_ID**: "I need the ID of the Notion page where the knowledge base should be synced. You can find it in the page URL (the 32-character hex string after the page title)."

Once provided, export them in the shell:
```bash
export NOTION_API_KEY="<value>"
export NOTION_ROOT_PAGE_ID="<value>"
```

### Step 2: Check dependencies

```bash
pip install notion-client python-frontmatter 2>/dev/null || pip3 install notion-client python-frontmatter
```

### Step 3: Run the sync

Ask the user which mode to use via `AskUserQuestion`:

| Mode | Command | When to use |
|---|---|---|
| **Dry run** (recommended first time) | `python scripts/notion_sync.py --dry-run` | Preview what will be created/updated without touching Notion |
| **Normal sync** | `python scripts/notion_sync.py` | Incremental sync — only changed files are updated |
| **Force sync** | `python scripts/notion_sync.py --force` | Re-sync everything, ignoring cached hashes |

### Step 4: Report results

After the sync completes, report to the CEO:
- How many pages/entries were created, updated, skipped
- Any errors that occurred
- The Notion root page URL for quick access

## How it works

The sync script (`scripts/notion_sync.py`):

- **Databases** are created for structured collections: hypotheses, experiments, findings, literature, features, investigations, implementations, retrospectives, and tasks. Each entry becomes a database row with typed properties (Status, Priority, Type, Impact, etc.)
- **Pages** are created for narrative content: mission files (CHALLENGE.md, BACKLOG.md, DECISIONS.md), reports, and articles.
- **INDEX.md** content is synced directly into the root page.
- **Change tracking** uses `kb/.notion-sync.json` to store content hashes and Notion page IDs. Only files that changed since the last sync are updated.
- **Overwrites on re-sync**: existing pages are updated in place (blocks deleted and re-created), never duplicated.

## Troubleshooting

- **"Integration not connected"**: The Notion integration must be shared with the target page. Go to the page in Notion → ··· → Connections → Add your integration.
- **API rate limits**: The script handles Notion's 100 blocks/request limit. For very large kb/ directories, the sync may take a few minutes.
- **Sync state reset**: Delete `kb/.notion-sync.json` to force a complete re-sync from scratch (all pages will be re-created).
