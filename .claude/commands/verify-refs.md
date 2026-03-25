# Verify References — Isolated Fact-Checking Pipeline

Run the full reference verification pipeline: audit claims completeness, extract cited claims from markdown documents, verify each against its original source using isolated agents, and reconcile results.

**Key principle**: Verification agents have ZERO project context. They only know "fetch this URL and check this claim." This breaks circular trust where errors in our literature notes propagate to documents unchecked.

## Setup

Determine the documents directory. Look for markdown files with `## References` sections in likely locations: `drafts/`, `docs/`, `articles/`, or the project root. Set `DOCS_DIR` to whichever directory contains the target documents.

Set `REFS_DIR` to `references/` (default output directory for the pipeline).

## Workflow

### Phase 0: Claims Completeness Audit (runs in parallel with Phase 1)

This phase uses the **claims-auditor** agent to identify unsourced assertions and overclaims — gaps that the citation-based pipeline (Phases 1-3) cannot catch.

```bash
python3 scripts/ref_audit.py generate $DOCS_DIR --output $REFS_DIR/audit_prompts.json
```

This produces one prompt per document. For each document:

1. Spawn a **claims-auditor** agent with the prompt (use `sonnet` model for speed)
2. The agent reads the full document and returns a JSON array of flagged assertions
3. Save each agent's JSON output to `$REFS_DIR/audit-{doc-slug}.json`

**Spawn all auditor agents in parallel** — they are independent per-document. This phase runs concurrently with Phase 1 extraction.

After all agents complete, generate the consolidated report:

```bash
python3 scripts/ref_audit.py apply $REFS_DIR --output $REFS_DIR/audit-report.md
```

Report to user: "Completeness audit: N overclaims, M unsourced across K documents. H high severity."

The audit report feeds into Phase 5 (Fix) alongside the verification report.

### Phase 1: Extract

Run the extraction script to parse all references and claims from documents:

```bash
python3 scripts/ref_extract.py $DOCS_DIR --output $REFS_DIR
```

Report to user: "Extracted N sources and M claims from K documents."

Review the output CSVs briefly. Spot-check a few claims for extraction quality.

### Phase 1.5: Triage (LLM-assisted filtering)

The regex extractor over-extracts — it catches editorial text, illustrative examples, and misattributes claims to wrong sources. The triage phase uses the **claim-reviewer** agent to filter and correct before verification.

```bash
python3 scripts/ref_triage.py generate --refs-dir $REFS_DIR --drafts-dir $DOCS_DIR --output $REFS_DIR/triage_prompts.json
```

For each triage batch:
1. Spawn the **claim-reviewer** agent with the prompt (it CAN read document files for context)
2. The agent classifies each claim as: VERIFY, VERIFY_REATTRIBUTE, EDITORIAL, or SYNTHESIS
3. Apply results: `python3 scripts/ref_triage.py apply --refs-dir $REFS_DIR triage_results.json`

This filters out editorial content (our own text) and synthesis (our interpretations), and corrects source misattributions. Only VERIFY and VERIFY_REATTRIBUTE claims proceed to Phase 2.

Report: "Triage complete: N claims to verify, M editorial filtered, K reattributed, J synthesis flagged."

### Phase 2: Verify

This is the critical phase. For each source with PENDING claims:

1. Read `$REFS_DIR/sources.csv` and `$REFS_DIR/claims.csv`
2. Group claims by source_id
3. For each source (in order of source_id):
   a. Generate an isolated prompt containing ONLY the URL and claims
   b. Spawn the **fact-checker** agent with that prompt
   c. The fact-checker has ONLY WebFetch and WebSearch — no project file access
   d. Parse the agent's JSON response
   e. Update claims.csv with results
   f. Report progress: "Source S{id} ({name}): {confirmed}/{contradicted}/{not_found}/{paywalled} of {total}"

**Important**: Generate prompts using `python3 scripts/ref_verify.py generate --refs-dir $REFS_DIR` which outputs a JSON array of {source_id, prompt, claim_ids}. Use each prompt as the fact-checker agent's input.

After each agent completes, save results and update the CSV. This way partial progress is preserved if the pipeline is interrupted.

To maximize throughput, spawn up to 3 fact-checker agents in parallel (for different sources). Wait for all to complete before spawning the next batch.

### Phase 3: Reconcile

After all sources are verified (or as many as possible):

```bash
python3 scripts/ref_reconcile.py --refs-dir $REFS_DIR --project-root .
```

This generates:
- `$REFS_DIR/verification-report.md` — Summary + all issues
- `$REFS_DIR/fix-suggestions.md` — Concrete fixes for contradicted claims

Present the verification report summary to the user. Highlight:
- Number of CONTRADICTED claims (these need fixes)
- Number of propagation errors (same error in document AND literature file)
- Number of PAYWALLED sources (need manual verification)

### Phase 4: Register (if running within the kb/ workflow)

If the project uses the kb/ knowledge base system:

1. Determine the next CR{NUM} from existing files in `kb/reports/`
2. Copy or summarize the verification report to `kb/reports/CR{NUM}-reference-verification.md`
3. Update `kb/INDEX.md` with the new report
4. Update BACKLOG.md last IDs if applicable

### Phase 5: Fix (merged, with user approval)

Merge results from **both** reports:
- `$REFS_DIR/verification-report.md` — fixes for CONTRADICTED cited claims (from Phases 1-3)
- `$REFS_DIR/audit-report.md` — fixes for unsourced and overclaimed assertions (from Phase 0)

For CONTRADICTED claims:
1. Present fix-suggestions.md to the user
2. Ask: "Apply these fixes to documents and literature files?"
3. If approved, apply fixes to documents and `kb/research/literature/` files
4. Re-run extraction and a targeted re-verification of fixed claims

For audit flags:
- **UNSOURCED (HIGH)** — specific number/stat: find a citation or soften the language
- **UNSOURCED (MEDIUM/LOW)** — general claim: determine if common knowledge; soften if borderline
- **OVERCLAIM** — soften language with qualifying hedges ("tends to", "in our assessment", "may"), remove absolutes ("systematically", "proven", "consensus")

## Notes

- The pipeline is idempotent. Re-running extraction overwrites CSVs. Re-running verification only processes PENDING claims.
- **Phase 0 runs in parallel with Phase 1.** They are independent — Phase 0 audits completeness (unsourced/overclaim), Phase 1+ audits accuracy (cited claims). Launch both simultaneously.
- **Phase 1.5 (Triage) is critical.** Without it, ~45% of extracted claims are false positives (editorial text, misattributions). Triage cuts verification agent work roughly in half and eliminates noise from the final report.
- Sources without URLs are verified via WebSearch for the title/author.
- x.com (Twitter) sources use WebSearch since direct fetch often fails.
- Paywalled sources (ACM, Springer) may only verify claims visible in the abstract.
- The **fact-checker** agent (`.claude/agents/fact-checker.md`) has ZERO project file access — isolation is the key design principle.
- The **claim-reviewer** agent (`.claude/agents/claim-reviewer.md`) CAN read document files — it needs paragraph context to judge whether a claim is editorial or sourced.
- The **claims-auditor** agent (`.claude/agents/claims-auditor.md`) CAN read document files — it needs the full article to assess completeness. Use `sonnet` model for speed.
- Phase 0 is advisory: LLM-based audits are non-deterministic and may produce false positives. Human review is expected before applying fixes. The audit report groups flags by severity to aid triage.
