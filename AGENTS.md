# Limina — Shared Runtime Contract

Limina is a research-first contract for autonomous technical investigation.

This file is the shared machine-facing instruction surface. Keep it short, specific, and stable. Runtime-specific details belong in adapters, skills, or scoped rules.

## Core Rules

1. Durable state lives in `kb/`. If it only exists in chat, it is not persistent.
2. The only required core workflow is `H -> E -> F`.
3. Do not create an experiment before the hypothesis exists.
4. Do not create a finding before the experiment exists.
5. `ACTIVE.md` is the only always-on state file. Keep it narrow.
6. Keep files small and specific. Prefer a concrete artifact over a generic note.
7. Store reusable lessons in `kb/lessons/` as small topic files. Read only the ones you need.
8. Use `CR` and `SR` only for real review points: major criticism, reset, or direction change.
9. If the evaluation, baseline, or prior state looks untrustworthy, stop optimizing and resolve that first.
10. Run `python3 scripts/kb_validate.py` after substantial kb edits and before closing kb-heavy work.
11. Every note in the research core must include a `## Links` section with real wikilinks.
12. Artifact notes alias their ID in frontmatter, so use `[[H001]]`, `[[E003]]`, `[[F010]]`, and similar links instead of raw file paths. Fixed notes use `[[ACTIVE]]`, `[[CHALLENGE]]`, and `[[DASHBOARD]]`.

## Session Start

At the start of a session:

1. Read `kb/mission/CHALLENGE.md`.
2. Read `kb/ACTIVE.md`.
3. Open only the linked artifacts you need next.
4. Search `kb/` before creating a new hypothesis, finding, or review.
5. If artifacts disagree, treat that inconsistency as a blocker until resolved.

## Working Protocol

- Update the current artifact's `Progress` section at stopping points.
- Update `kb/ACTIVE.md` whenever the objective, next step, blocker, or working set changes.
- Persist durable conclusions in artifacts, not only in chat.
- Ask the user early when blocked on data, access, decisions, or trust in the evaluation.
- Keep reviews scoped and evidence-backed.
- Prefer `python3 scripts/kb_new_artifact.py ...` when creating a new core artifact.
- When you create a child artifact, ensure the parent note links back to it.

## Artifact Model

The validator-enforced research core is:

- `H` hypotheses in `kb/research/hypotheses/`
- `E` experiments in `kb/research/experiments/`
- `F` findings in `kb/research/findings/`
- `L` literature notes in `kb/research/literature/`
- `CR` challenge reviews in `kb/reports/`
- `SR` strategic reviews in `kb/reports/`

Required non-ID files:

- `kb/ACTIVE.md`
- `kb/mission/CHALLENGE.md`

## ID Allocation

Do not maintain manual counters in prompt-loaded files.

Use:

```bash
python3 scripts/kb_next_id.py H
python3 scripts/kb_new_artifact.py H "Hypothesis title"
```

Replace `H` with `E`, `F`, `L`, `CR`, or `SR`.

## Research Quality

When writing a hypothesis or conclusion, be explicit about:

- the mechanism you think matters
- the evidence you actually have
- why the claim should generalize beyond the current eval slice
- what could still be a shortcut, leak, or domain-specific patch

Do not present benchmark gains as mission progress unless the underlying capability is credible.

## Communication

The active session is the default transport for updates.

- Use concise milestone summaries.
- Ask direct questions when blocked.
- Persist anything that must survive context loss.

Implementation work can happen, but it is not a parallel core artifact graph in Limina. Research drives the contract; delivery details belong to the local project, not the shared template.
