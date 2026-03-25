---
name: verify-refs
description: Verify references and claims in markdown documents. Use when asked to fact-check citations, audit claims completeness, or run a reference verification pipeline on draft articles or documentation with inline references.
---

# Verify References

Automated reference verification pipeline for markdown documents with inline citations. Catches both **accuracy errors** (cited claims that don't match the source) and **completeness gaps** (unsourced assertions and overclaims).

## What It Does

Two complementary verification passes run in parallel:

**Completeness audit (Phase 0)** — LLM-based review of each document to flag:
- Unsourced factual assertions (specific numbers, dates, comparisons without citations)
- Overclaims (assertions that go beyond what cited evidence supports)

**Citation verification (Phases 1-3)** — Automated pipeline that:
1. Extracts all inline claims (percentages, ratios, quotes, attributions) with regex
2. Triages extracted claims (LLM-assisted filtering of editorial text and synthesis)
3. Fact-checks each claim against its original source using isolated agents
4. Reconciles results and detects propagation errors (same error in docs AND literature files)

## Prerequisites

- Markdown documents with a `## References` section containing sourced citations
- References in the format: `- Author. "Title." Year. [label](url)`
- Inline citations as parenthetical references: `(Author, Year)` or attribution phrases

## Components

| Component | Location | Purpose |
|---|---|---|
| `/verify-refs` command | `.claude/commands/verify-refs.md` | Orchestration workflow |
| `ref_extract.py` | `scripts/` | Extract references and claims from markdown |
| `ref_triage.py` | `scripts/` | LLM-assisted claim filtering |
| `ref_verify.py` | `scripts/` | Generate isolated fact-checking prompts |
| `ref_reconcile.py` | `scripts/` | Post-verification analysis and reporting |
| `ref_audit.py` | `scripts/` | Claims completeness audit (Phase 0) |
| `fact-checker` agent | `.claude/agents/` | Isolated source verification (no project access) |
| `claim-reviewer` agent | `.claude/agents/` | Claim triage and classification |
| `claims-auditor` agent | `.claude/agents/` | Completeness audit per document |

## Output

All output goes to `references/`:
- `sources.csv` — unique sources extracted from documents
- `claims.csv` — all verifiable claims with verification status
- `verification-report.md` — accuracy report (contradicted, not found, paywalled, confirmed)
- `fix-suggestions.md` — concrete fixes for contradicted claims
- `audit-report.md` — completeness report (unsourced and overclaimed assertions)

## Usage

Run the full pipeline via the slash command:

```
/verify-refs
```

Or run individual phases:

```bash
# Phase 0: Completeness audit
python3 scripts/ref_audit.py generate docs/ --output references/audit_prompts.json

# Phase 1: Extract
python3 scripts/ref_extract.py docs/ --output references/

# Phase 1.5: Triage
python3 scripts/ref_triage.py generate --refs-dir references/ --drafts-dir docs/

# Phase 2: Verify (generates prompts for fact-checker agents)
python3 scripts/ref_verify.py generate --refs-dir references/

# Phase 3: Reconcile
python3 scripts/ref_reconcile.py --refs-dir references/ --project-root .
```

## Design Principles

- **Isolated verification**: Fact-checker agents have zero project context — they only see a URL and claims. This prevents circular trust where errors in research notes propagate to documents unchecked.
- **Idempotent**: Re-running phases preserves prior work. Verification only processes PENDING claims.
- **Stdlib-only scripts**: No external dependencies for the core pipeline scripts.
- **Advisory audit**: The LLM-based completeness audit (Phase 0) is non-deterministic. It flags potential issues for human review, not automated correction.
