#!/usr/bin/env python3
"""
Sync kb/ knowledge base to Notion.

Mirrors the kb/ directory structure into Notion pages and databases.
Uses a local mapping file (kb/.notion-sync.json) to track which files
map to which Notion pages, enabling updates instead of duplicates.

Usage:
    python scripts/notion_sync.py [--dry-run] [--force]

Environment variables:
    NOTION_API_KEY       - Notion integration token (required)
    NOTION_ROOT_PAGE_ID  - Root page ID in Notion to sync under (required)
    KB_PATH              - Path to kb/ directory (default: ./kb)

Requirements:
    pip install notion-client python-frontmatter
"""

import argparse
import hashlib
import json
import os
import re
import sys
from pathlib import Path
from textwrap import dedent

try:
    from notion_client import Client
except ImportError:
    print("Error: notion-client not installed. Run: pip install notion-client python-frontmatter")
    sys.exit(1)

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

SYNC_FILE = ".notion-sync.json"
MAX_BLOCKS_PER_REQUEST = 100
MAX_TEXT_LENGTH = 2000

# Directories that should become Notion databases (with their property schemas)
DATABASE_DIRS = {
    "research/hypotheses": {
        "emoji": "🧪",
        "title_prop": "Statement",
        "properties": {
            "ID": {"rich_text": {}},
            "Status": {
                "select": {
                    "options": [
                        {"name": "PROPOSED", "color": "gray"},
                        {"name": "TESTING", "color": "blue"},
                        {"name": "CONFIRMED", "color": "green"},
                        {"name": "REJECTED", "color": "red"},
                    ]
                }
            },
        },
    },
    "research/experiments": {
        "emoji": "🔬",
        "title_prop": "Experiment",
        "properties": {
            "ID": {"rich_text": {}},
            "Status": {
                "select": {
                    "options": [
                        {"name": "DESIGNED", "color": "gray"},
                        {"name": "RUNNING", "color": "blue"},
                        {"name": "COMPLETED", "color": "green"},
                        {"name": "FAILED", "color": "red"},
                    ]
                }
            },
            "Hypothesis": {"rich_text": {}},
        },
    },
    "research/findings": {
        "emoji": "💡",
        "title_prop": "Finding",
        "properties": {
            "ID": {"rich_text": {}},
            "Impact": {
                "select": {
                    "options": [
                        {"name": "HIGH", "color": "red"},
                        {"name": "MEDIUM", "color": "yellow"},
                        {"name": "LOW", "color": "gray"},
                    ]
                }
            },
        },
    },
    "research/literature": {
        "emoji": "📚",
        "title_prop": "Title",
        "properties": {
            "ID": {"rich_text": {}},
            "Type": {
                "select": {
                    "options": [
                        {"name": "PAPER", "color": "blue"},
                        {"name": "BLOG", "color": "green"},
                        {"name": "REPO", "color": "purple"},
                        {"name": "BENCHMARK", "color": "orange"},
                        {"name": "SURVEY", "color": "gray"},
                    ]
                }
            },
            "Relevance": {
                "select": {
                    "options": [
                        {"name": "HIGH", "color": "red"},
                        {"name": "MEDIUM", "color": "yellow"},
                        {"name": "LOW", "color": "gray"},
                    ]
                }
            },
        },
    },
}

# Directories/files that become regular pages
PAGE_STRUCTURE = {
    "mission": {"emoji": "🎯", "title": "Mission"},
    "research": {"emoji": "🔬", "title": "Research"},
    "reports": {"emoji": "📊", "title": "Reports"},
    "lessons": {"emoji": "🧠", "title": "Lessons"},
    "articles": {"emoji": "📝", "title": "Articles"},
}


# ---------------------------------------------------------------------------
# Markdown → Notion block converter
# ---------------------------------------------------------------------------


def md_to_rich_text(text: str) -> list[dict]:
    """Convert inline markdown to Notion rich text array."""
    segments = []
    i = 0
    while i < len(text):
        # Bold + italic
        m = re.match(r"\*\*\*(.+?)\*\*\*", text[i:])
        if m:
            segments.append(
                {
                    "type": "text",
                    "text": {"content": m.group(1)},
                    "annotations": {"bold": True, "italic": True},
                }
            )
            i += m.end()
            continue

        # Bold
        m = re.match(r"\*\*(.+?)\*\*", text[i:])
        if m:
            segments.append(
                {
                    "type": "text",
                    "text": {"content": m.group(1)},
                    "annotations": {"bold": True},
                }
            )
            i += m.end()
            continue

        # Italic
        m = re.match(r"\*(.+?)\*", text[i:])
        if m:
            segments.append(
                {
                    "type": "text",
                    "text": {"content": m.group(1)},
                    "annotations": {"italic": True},
                }
            )
            i += m.end()
            continue

        # Inline code
        m = re.match(r"`([^`]+)`", text[i:])
        if m:
            segments.append(
                {
                    "type": "text",
                    "text": {"content": m.group(1)},
                    "annotations": {"code": True},
                }
            )
            i += m.end()
            continue

        # Link
        m = re.match(r"\[([^\]]+)\]\(([^)]+)\)", text[i:])
        if m:
            segments.append(
                {
                    "type": "text",
                    "text": {"content": m.group(1), "link": {"url": m.group(2)}},
                }
            )
            i += m.end()
            continue

        # Plain text — collect until next special char
        m = re.match(r"[^*`\[]+", text[i:])
        if m:
            segments.append(
                {"type": "text", "text": {"content": m.group(0)}}
            )
            i += m.end()
            continue

        # Fallback: single char
        segments.append({"type": "text", "text": {"content": text[i]}})
        i += 1

    return segments if segments else [{"type": "text", "text": {"content": ""}}]


def truncate_rich_text(rich_text: list[dict]) -> list[dict]:
    """Ensure no single text segment exceeds Notion's limit."""
    result = []
    for seg in rich_text:
        content = seg.get("text", {}).get("content", "")
        if len(content) > MAX_TEXT_LENGTH:
            for j in range(0, len(content), MAX_TEXT_LENGTH):
                chunk = dict(seg)
                chunk["text"] = dict(seg["text"])
                chunk["text"]["content"] = content[j : j + MAX_TEXT_LENGTH]
                result.append(chunk)
        else:
            result.append(seg)
    return result


def md_to_blocks(content: str) -> list[dict]:
    """Convert markdown content to Notion API blocks."""
    blocks = []
    lines = content.split("\n")
    i = 0

    while i < len(lines):
        line = lines[i]

        # Skip empty lines
        if not line.strip():
            i += 1
            continue

        # Horizontal rule
        if re.match(r"^---+\s*$", line):
            blocks.append({"type": "divider", "divider": {}})
            i += 1
            continue

        # Headings
        m = re.match(r"^(#{1,3})\s+(.+)$", line)
        if m:
            level = len(m.group(1))
            text = m.group(2).strip()
            block_type = f"heading_{level}"
            blocks.append(
                {
                    "type": block_type,
                    block_type: {"rich_text": truncate_rich_text(md_to_rich_text(text))},
                }
            )
            i += 1
            continue

        # Code block
        m = re.match(r"^```(\w*)$", line)
        if m:
            lang = m.group(1) or "plain text"
            code_lines = []
            i += 1
            while i < len(lines) and not lines[i].strip().startswith("```"):
                code_lines.append(lines[i])
                i += 1
            i += 1  # skip closing ```
            code_content = "\n".join(code_lines)
            # Notion code blocks have 2000 char limit
            if len(code_content) > MAX_TEXT_LENGTH:
                code_content = code_content[:MAX_TEXT_LENGTH - 20] + "\n... (truncated)"
            blocks.append(
                {
                    "type": "code",
                    "code": {
                        "rich_text": [
                            {"type": "text", "text": {"content": code_content}}
                        ],
                        "language": lang if lang in NOTION_LANGUAGES else "plain text",
                    },
                }
            )
            continue

        # Blockquote
        m = re.match(r"^>\s*(.*)$", line)
        if m:
            quote_lines = [m.group(1)]
            i += 1
            while i < len(lines) and re.match(r"^>\s*(.*)$", lines[i]):
                quote_lines.append(re.match(r"^>\s*(.*)$", lines[i]).group(1))
                i += 1
            quote_text = " ".join(quote_lines)
            blocks.append(
                {
                    "type": "callout",
                    "callout": {
                        "rich_text": truncate_rich_text(md_to_rich_text(quote_text)),
                        "icon": {"emoji": "💡"},
                    },
                }
            )
            continue

        # Table
        if "|" in line and i + 1 < len(lines) and re.match(r"^\|[-\s|:]+\|$", lines[i + 1]):
            table_rows = []
            # Header row
            cells = [c.strip() for c in line.split("|")[1:-1]]
            table_rows.append(cells)
            i += 1  # skip separator
            i += 1
            while i < len(lines) and "|" in lines[i] and lines[i].strip().startswith("|"):
                cells = [c.strip() for c in lines[i].split("|")[1:-1]]
                table_rows.append(cells)
                i += 1
            if table_rows:
                width = max(len(row) for row in table_rows)
                notion_rows = []
                for row in table_rows:
                    # Pad rows to same width
                    padded = row + [""] * (width - len(row))
                    notion_row = {
                        "type": "table_row",
                        "table_row": {
                            "cells": [
                                truncate_rich_text(md_to_rich_text(cell))
                                for cell in padded
                            ]
                        },
                    }
                    notion_rows.append(notion_row)
                blocks.append(
                    {
                        "type": "table",
                        "table": {
                            "table_width": width,
                            "has_column_header": True,
                            "has_row_header": False,
                            "children": notion_rows,
                        },
                    }
                )
            continue

        # Bullet list
        m = re.match(r"^(\s*)[-*]\s+(.+)$", line)
        if m:
            text = m.group(2)
            blocks.append(
                {
                    "type": "bulleted_list_item",
                    "bulleted_list_item": {
                        "rich_text": truncate_rich_text(md_to_rich_text(text))
                    },
                }
            )
            i += 1
            continue

        # Numbered list
        m = re.match(r"^\s*\d+\.\s+(.+)$", line)
        if m:
            text = m.group(1)
            blocks.append(
                {
                    "type": "numbered_list_item",
                    "numbered_list_item": {
                        "rich_text": truncate_rich_text(md_to_rich_text(text))
                    },
                }
            )
            i += 1
            continue

        # Checkbox
        m = re.match(r"^- \[([ xX])\]\s+(.+)$", line)
        if m:
            checked = m.group(1).lower() == "x"
            text = m.group(2)
            blocks.append(
                {
                    "type": "to_do",
                    "to_do": {
                        "rich_text": truncate_rich_text(md_to_rich_text(text)),
                        "checked": checked,
                    },
                }
            )
            i += 1
            continue

        # HTML comment (skip)
        if line.strip().startswith("<!--"):
            while i < len(lines) and "-->" not in lines[i]:
                i += 1
            i += 1
            continue

        # Regular paragraph
        para_lines = [line.strip()]
        i += 1
        while (
            i < len(lines)
            and lines[i].strip()
            and not re.match(r"^#{1,3}\s", lines[i])
            and not re.match(r"^[-*]\s", lines[i])
            and not re.match(r"^\d+\.\s", lines[i])
            and not re.match(r"^```", lines[i])
            and not re.match(r"^>\s", lines[i])
            and not re.match(r"^---+\s*$", lines[i])
            and not re.match(r"^\|", lines[i])
            and not re.match(r"^- \[", lines[i])
            and not lines[i].strip().startswith("<!--")
        ):
            para_lines.append(lines[i].strip())
            i += 1
        para_text = " ".join(para_lines)
        if para_text.strip():
            blocks.append(
                {
                    "type": "paragraph",
                    "paragraph": {
                        "rich_text": truncate_rich_text(md_to_rich_text(para_text))
                    },
                }
            )

    return blocks


# Notion supported languages for code blocks
NOTION_LANGUAGES = {
    "python", "javascript", "typescript", "bash", "shell", "sql", "json",
    "yaml", "toml", "html", "css", "markdown", "rust", "go", "java",
    "c", "cpp", "c++", "ruby", "php", "swift", "kotlin", "scala",
    "r", "matlab", "dockerfile", "graphql", "plain text", "mermaid",
}


# ---------------------------------------------------------------------------
# Metadata extraction from markdown front matter
# ---------------------------------------------------------------------------


def extract_front_matter(content: str) -> tuple[dict, str]:
    """Extract YAML-like front matter and return (metadata, body)."""
    metadata = {}
    body = content

    # Check for > **Key**: Value pattern (our kb format)
    lines = content.split("\n")
    body_start = 0
    for idx, line in enumerate(lines):
        m = re.match(r"^>\s*\*\*(\w[\w\s]*)\*\*:\s*(.+)$", line)
        if m:
            key = m.group(1).strip().lower()
            value = m.group(2).strip()
            metadata[key] = value
            body_start = idx + 1
        elif line.strip() and not line.startswith(">") and idx > 0:
            break

    if metadata:
        body = "\n".join(lines[body_start:])

    return metadata, body


def extract_id_from_filename(filename: str) -> str:
    """Extract ID like H001, E003, F017 from filename."""
    m = re.match(r"^([A-Z]+\d+)", filename)
    return m.group(1) if m else ""


def extract_title_from_content(content: str) -> str:
    """Extract the first H1 heading as title."""
    m = re.search(r"^#\s+(.+)$", content, re.MULTILINE)
    if m:
        # Remove the ID prefix if present (e.g., "# H001 — Title" -> "Title")
        title = m.group(1).strip()
        title = re.sub(r"^[A-Z]+\d+\s*[—–-]\s*", "", title)
        return title
    return ""


# ---------------------------------------------------------------------------
# Notion Sync Engine
# ---------------------------------------------------------------------------


class NotionSync:
    def __init__(self, api_key: str, root_page_id: str, kb_path: str, dry_run: bool = False, force: bool = False):
        self.client = Client(auth=api_key)
        self.root_page_id = root_page_id
        self.kb_path = Path(kb_path)
        self.dry_run = dry_run
        self.force = force
        self.sync_file = self.kb_path / SYNC_FILE
        self.sync_state = self._load_sync_state()
        self.stats = {"created": 0, "updated": 0, "skipped": 0, "errors": 0}

    def _load_sync_state(self) -> dict:
        if self.sync_file.exists():
            with open(self.sync_file) as f:
                return json.load(f)
        return {"pages": {}, "databases": {}, "sections": {}}

    def _save_sync_state(self):
        if not self.dry_run:
            with open(self.sync_file, "w") as f:
                json.dump(self.sync_state, f, indent=2)

    def _file_hash(self, path: Path) -> str:
        return hashlib.md5(path.read_text(encoding="utf-8").encode()).hexdigest()

    def _needs_update(self, file_path: str, current_hash: str) -> bool:
        if self.force:
            return True
        stored = self.sync_state["pages"].get(file_path, {})
        return stored.get("hash") != current_hash

    # -- Page operations --

    def _create_page(self, parent_id: str, title: str, blocks: list[dict], emoji: str = None) -> str:
        """Create a Notion page and return its ID."""
        if self.dry_run:
            print(f"  [DRY RUN] Would create page: {title}")
            return "dry-run-id"

        page_data = {
            "parent": {"page_id": parent_id},
            "properties": {"title": [{"text": {"content": title}}]},
        }
        if emoji:
            page_data["icon"] = {"emoji": emoji}

        page = self.client.pages.create(**page_data)
        page_id = page["id"]

        # Add blocks in chunks
        for j in range(0, len(blocks), MAX_BLOCKS_PER_REQUEST):
            chunk = blocks[j : j + MAX_BLOCKS_PER_REQUEST]
            self.client.blocks.children.append(block_id=page_id, children=chunk)

        return page_id

    def _update_page_content(self, page_id: str, blocks: list[dict]):
        """Replace all content in an existing page."""
        if self.dry_run:
            print(f"  [DRY RUN] Would update page: {page_id}")
            return

        # Delete existing blocks
        existing = self.client.blocks.children.list(block_id=page_id)
        for block in existing["results"]:
            try:
                self.client.blocks.delete(block_id=block["id"])
            except Exception:
                pass  # Some blocks can't be deleted

        # Add new blocks
        for j in range(0, len(blocks), MAX_BLOCKS_PER_REQUEST):
            chunk = blocks[j : j + MAX_BLOCKS_PER_REQUEST]
            self.client.blocks.children.append(block_id=page_id, children=chunk)

    # -- Database operations --

    def _create_database(self, parent_id: str, title: str, schema: dict) -> str:
        """Create a Notion database and return its ID."""
        if self.dry_run:
            print(f"  [DRY RUN] Would create database: {title}")
            return "dry-run-id"

        properties = {"Name": {"title": {}}}
        properties.update(schema.get("properties", {}))

        db_data = {
            "parent": {"page_id": parent_id},
            "title": [{"text": {"content": title}}],
            "properties": properties,
        }
        if schema.get("emoji"):
            db_data["icon"] = {"emoji": schema["emoji"]}

        db = self.client.databases.create(**db_data)
        return db["id"]

    def _create_or_update_db_entry(self, database_id: str, title: str, props: dict, blocks: list[dict], entry_key: str):
        """Create or update a database entry (page within a database)."""
        existing_page_id = self.sync_state["pages"].get(entry_key, {}).get("notion_id")

        properties = {"Name": {"title": [{"text": {"content": title}}]}}

        # Map our extracted props to Notion property formats
        for key, value in props.items():
            if isinstance(value, str):
                if key in ("Status", "Impact", "Type", "Relevance", "Priority"):
                    properties[key] = {"select": {"name": value}}
                elif key == "ID":
                    properties[key] = {"rich_text": [{"text": {"content": value}}]}
                else:
                    properties[key] = {"rich_text": [{"text": {"content": value}}]}

        if existing_page_id and not self.dry_run:
            try:
                # Update properties
                self.client.pages.update(page_id=existing_page_id, properties=properties)
                # Update content
                self._update_page_content(existing_page_id, blocks)
                self.stats["updated"] += 1
                print(f"  ✓ Updated: {title}")
                return existing_page_id
            except Exception as e:
                print(f"  ⚠ Update failed, recreating: {e}")

        if self.dry_run:
            print(f"  [DRY RUN] Would create entry: {title}")
            return "dry-run-id"

        page_data = {
            "parent": {"database_id": database_id},
            "properties": properties,
        }
        page = self.client.pages.create(**page_data)
        page_id = page["id"]

        # Add content blocks
        for j in range(0, len(blocks), MAX_BLOCKS_PER_REQUEST):
            chunk = blocks[j : j + MAX_BLOCKS_PER_REQUEST]
            self.client.blocks.children.append(block_id=page_id, children=chunk)

        self.stats["created"] += 1
        print(f"  ✓ Created: {title}")
        return page_id

    # -- Sync logic --

    def sync(self):
        """Run full sync of kb/ to Notion."""
        print(f"\n{'='*60}")
        print(f"  Syncing kb/ → Notion")
        print(f"  KB path: {self.kb_path}")
        print(f"  Root page: {self.root_page_id}")
        print(f"  Mode: {'DRY RUN' if self.dry_run else 'LIVE'}")
        print(f"{'='*60}\n")

        # 1. Sync ACTIVE.md as the root page content
        self._sync_active()

        # 2. Create section pages
        for section, config in PAGE_STRUCTURE.items():
            section_path = self.kb_path / section
            if section_path.exists():
                self._sync_section(section, config)

        # 3. Sync databases (collections of files)
        for dir_key, schema in DATABASE_DIRS.items():
            dir_path = self.kb_path / dir_key
            if dir_path.exists() and any(dir_path.glob("*.md")):
                self._sync_database_dir(dir_key, schema)

        # 4. Sync standalone files in sections
        for section in PAGE_STRUCTURE:
            section_path = self.kb_path / section
            if section_path.exists():
                self._sync_standalone_files(section, section_path)

        self._save_sync_state()

        print(f"\n{'='*60}")
        print(f"  Sync complete!")
        print(f"  Created: {self.stats['created']}")
        print(f"  Updated: {self.stats['updated']}")
        print(f"  Skipped: {self.stats['skipped']}")
        print(f"  Errors:  {self.stats['errors']}")
        print(f"{'='*60}\n")

    def _sync_active(self):
        """Sync ACTIVE.md as content on the root page."""
        active_path = self.kb_path / "ACTIVE.md"
        if not active_path.exists():
            return

        print("📑 Syncing ACTIVE.md to root page...")
        content = active_path.read_text(encoding="utf-8")
        file_hash = self._file_hash(active_path)
        file_key = "ACTIVE.md"

        if not self._needs_update(file_key, file_hash):
            print("  ⏭ Skipped (unchanged)")
            self.stats["skipped"] += 1
            return

        blocks = md_to_blocks(content)
        if not self.dry_run:
            self._update_page_content(self.root_page_id, blocks)
            self.stats["updated"] += 1
            print("  ✓ Updated root page with ACTIVE.md")
        else:
            print("  [DRY RUN] Would update root page")

        self.sync_state["pages"][file_key] = {
            "notion_id": self.root_page_id,
            "hash": file_hash,
        }

    def _sync_section(self, section: str, config: dict):
        """Ensure a section page exists under root."""
        section_key = f"_section_{section}"

        if section_key in self.sync_state["sections"]:
            return self.sync_state["sections"][section_key]

        print(f"\n{config['emoji']} Creating section: {config['title']}")
        page_id = self._create_page(
            self.root_page_id,
            config["title"],
            [],  # Empty page, content added by sub-pages
            emoji=config["emoji"],
        )
        self.sync_state["sections"][section_key] = page_id
        return page_id

    def _get_section_id(self, section: str) -> str:
        section_key = f"_section_{section}"
        return self.sync_state["sections"].get(section_key, self.root_page_id)

    def _sync_database_dir(self, dir_key: str, schema: dict):
        """Sync a directory of files as a Notion database."""
        dir_path = self.kb_path / dir_key
        # Root-level dirs go under root page; nested dirs go under their section
        if "/" in dir_key:
            section = dir_key.split("/")[0]  # "research" or "engineering"
            parent_id = self._get_section_id(section)
        else:
            parent_id = self.root_page_id
        db_name = (dir_key.split("/")[1] if "/" in dir_key else dir_key).replace("_", " ").title()

        print(f"\n{schema['emoji']} Syncing database: {db_name}")

        # Create or get database
        db_key = f"_db_{dir_key}"
        if db_key in self.sync_state["databases"]:
            db_id = self.sync_state["databases"][db_key]
        else:
            db_id = self._create_database(parent_id, db_name, schema)
            self.sync_state["databases"][db_key] = db_id

        # Sync each .md file as a database entry
        for md_file in sorted(dir_path.glob("*.md")):
            if md_file.name.startswith("."):
                continue

            file_key = str(md_file.relative_to(self.kb_path))
            file_hash = self._file_hash(md_file)

            if not self._needs_update(file_key, file_hash):
                self.stats["skipped"] += 1
                continue

            content = md_file.read_text(encoding="utf-8")
            metadata, body = extract_front_matter(content)

            # Extract properties
            file_id = extract_id_from_filename(md_file.stem)
            title = extract_title_from_content(content) or md_file.stem
            blocks = md_to_blocks(body)

            props = {"ID": file_id}
            if "status" in metadata:
                props["Status"] = metadata["status"]
            if "impact" in metadata:
                props["Impact"] = metadata["impact"]
            if "type" in metadata:
                props["Type"] = metadata["type"]
            if "relevance" in metadata:
                props["Relevance"] = metadata["relevance"]
            if "priority" in metadata:
                props["Priority"] = metadata["priority"]
            if "linked artifacts" in metadata:
                props["Linked"] = metadata["linked artifacts"]

            try:
                page_id = self._create_or_update_db_entry(
                    db_id, title, props, blocks, file_key
                )
                self.sync_state["pages"][file_key] = {
                    "notion_id": page_id,
                    "hash": file_hash,
                }
            except Exception as e:
                print(f"  ✗ Error syncing {md_file.name}: {e}")
                self.stats["errors"] += 1

    def _sync_standalone_files(self, section: str, section_path: Path):
        """Sync standalone .md files (not in database dirs) as pages."""
        parent_id = self._get_section_id(section)

        # Get files directly in this section (not in subdirs that are databases)
        db_subdirs = {
            d.split("/")[1]
            for d in DATABASE_DIRS
            if d.startswith(section + "/")
        }

        for md_file in sorted(section_path.glob("*.md")):
            if md_file.name.startswith("."):
                continue

            file_key = str(md_file.relative_to(self.kb_path))
            file_hash = self._file_hash(md_file)

            if not self._needs_update(file_key, file_hash):
                self.stats["skipped"] += 1
                continue

            content = md_file.read_text(encoding="utf-8")
            title = extract_title_from_content(content) or md_file.stem.replace("-", " ").title()
            blocks = md_to_blocks(content)

            existing = self.sync_state["pages"].get(file_key, {}).get("notion_id")

            try:
                if existing and not self.dry_run:
                    self._update_page_content(existing, blocks)
                    self.stats["updated"] += 1
                    print(f"  ✓ Updated: {title}")
                    page_id = existing
                else:
                    page_id = self._create_page(parent_id, title, blocks)
                    if not self.dry_run:
                        self.stats["created"] += 1
                        print(f"  ✓ Created: {title}")

                self.sync_state["pages"][file_key] = {
                    "notion_id": page_id,
                    "hash": file_hash,
                }
            except Exception as e:
                print(f"  ✗ Error syncing {md_file.name}: {e}")
                self.stats["errors"] += 1

        # Recurse into subdirs that aren't databases
        for subdir in sorted(section_path.iterdir()):
            if subdir.is_dir() and subdir.name not in db_subdirs and not subdir.name.startswith("."):
                # Create a sub-page for this directory
                subdir_key = f"_section_{section}/{subdir.name}"
                if subdir_key not in self.sync_state["sections"]:
                    sub_page_id = self._create_page(
                        parent_id,
                        subdir.name.replace("-", " ").replace("_", " ").title(),
                        [],
                        emoji="📂",
                    )
                    self.sync_state["sections"][subdir_key] = sub_page_id

                sub_parent = self.sync_state["sections"][subdir_key]
                for md_file in sorted(subdir.glob("*.md")):
                    if md_file.name.startswith("."):
                        continue
                    file_key = str(md_file.relative_to(self.kb_path))
                    file_hash = self._file_hash(md_file)
                    if not self._needs_update(file_key, file_hash):
                        self.stats["skipped"] += 1
                        continue
                    content = md_file.read_text(encoding="utf-8")
                    title = extract_title_from_content(content) or md_file.stem.replace("-", " ").title()
                    blocks = md_to_blocks(content)
                    existing = self.sync_state["pages"].get(file_key, {}).get("notion_id")
                    try:
                        if existing and not self.dry_run:
                            self._update_page_content(existing, blocks)
                            self.stats["updated"] += 1
                            print(f"  ✓ Updated: {title}")
                            page_id = existing
                        else:
                            page_id = self._create_page(sub_parent, title, blocks)
                            if not self.dry_run:
                                self.stats["created"] += 1
                                print(f"  ✓ Created: {title}")
                        self.sync_state["pages"][file_key] = {
                            "notion_id": page_id,
                            "hash": file_hash,
                        }
                    except Exception as e:
                        print(f"  ✗ Error syncing {md_file.name}: {e}")
                        self.stats["errors"] += 1


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def main():
    parser = argparse.ArgumentParser(description="Sync kb/ to Notion")
    parser.add_argument("--dry-run", action="store_true", help="Show what would happen without making changes")
    parser.add_argument("--force", action="store_true", help="Force re-sync all files (ignore hashes)")
    parser.add_argument("--kb-path", default=None, help="Path to kb/ directory")
    args = parser.parse_args()

    api_key = os.environ.get("NOTION_API_KEY")
    root_page_id = os.environ.get("NOTION_ROOT_PAGE_ID")
    kb_path = args.kb_path or os.environ.get("KB_PATH", "./kb")

    if not api_key:
        print("Error: NOTION_API_KEY environment variable not set")
        sys.exit(1)
    if not root_page_id:
        print("Error: NOTION_ROOT_PAGE_ID environment variable not set")
        sys.exit(1)

    syncer = NotionSync(api_key, root_page_id, kb_path, dry_run=args.dry_run, force=args.force)
    syncer.sync()


if __name__ == "__main__":
    main()
