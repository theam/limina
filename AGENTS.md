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

## Mission First

Research exists to improve the mission, not to produce activity.

- Optimize for decision-quality progress, not local artifact completion.
- Treat benchmark or eval wins as mission progress only when the underlying capability is credible.
- Prefer mechanisms that should generalize beyond the current slice, wording, customer, or benchmark.
- If the current eval, baseline, or assumptions are not trustworthy, fix that before optimizing further.
- If the problem changes, update the research graph to reflect the new reality instead of forcing continuity.

## Working With The User

The user is the source of challenge framing, domain context, priorities, constraints, access, and missing information.

- Ask early for missing data, access, tools, context, decisions, or clarifications that materially affect the research direction.
- Do not stay silently blocked behind weak assumptions. Surface the uncertainty before building on it.
- When asking, state what you need, why it matters now, and what trade-off or degraded path exists if you do not get it.
- Ask narrowly scoped questions that unblock the next decision, not broad open-ended interviews.

## Session Recovery

Use this at the start of a session, after compaction, or whenever the current context may be partial or stale.

1. Read `kb/mission/CHALLENGE.md`.
2. Read `kb/ACTIVE.md`.
3. Open only the linked artifacts needed to reconstruct the current working state. Do not browse unrelated notes until the active scope is clear.
4. Reconstruct the current state before acting: current objective, next step, blocker, latest decisive evidence, and open decision.
5. Cross-check key numbers, dates, baselines, and decisions across the artifacts you read. If they disagree, treat that inconsistency as a blocker until resolved.
6. Search `kb/` before creating a new hypothesis, finding, or review.
7. Only continue once the active scope and state are coherent.

## Default Research Loop

Use this loop unless the current task clearly requires something else:

1. Frame the current decision, bottleneck, or uncertainty.
2. Identify the highest-value unknown blocking progress.
3. Search by mechanism and failure mode, not only by task label.
4. Create or revise `H` so it states a falsifiable claim, why it should work, why it might generalize, and what shortcut risks remain.
5. Design and run one decisive `E` with a fair baseline, explicit success criteria, and clear guardrails.
6. Write `F` as what the evidence actually established, what improved for real, and what debt remains.
7. Update `kb/ACTIVE.md` with the new objective, next step, blocker, and working set.
8. Escalate to `CR` or `SR` only if the direction, trust in the setup, or strategic framing has changed.

## Skill Routing

Before major work, identify the dominant bottleneck and pick the narrowest skill that matches it.

- Use `$exploratory-sota-research` when the main gap is external landscape search, mechanism discovery, benchmark mapping, or reproducibility scanning.
- Use `$experiment-rigor` when the main gap is hypothesis quality, experiment design, baseline fairness, metrics, guardrails, or interpreting whether a result is valid, invalid, or inconclusive.
- Use `$research-devil-advocate` when the main gap is adversarial review of the current direction: continue, continue with fixes, pivot, stop, or escalate.
- Use `$build-maintainable-software` when the main work is implementation, refactor, API or module design, maintainability review, or safe changeability.
- Default to one primary skill. Add a second only when the output of the first naturally hands off to a different kind of work.
- After using a skill, persist any durable result back into `kb/` and update `kb/ACTIVE.md` if the objective, next step, blocker, or working set changed.

## Evidence Quality

Default to these standards:

- Search by challenge, bottleneck, and mechanism family.
- Prefer generalizable approaches over narrow patches unless a narrow patch is the explicit goal.
- Compare against the strongest fair baseline you can afford.
- Control non-essential variables so the result can change a decision.
- Distinguish `valid negative`, `invalid test`, `implementation failure`, `insufficient signal`, and `trade-off failure`.
- Be explicit about mechanism, evidence, generalization, and shortcut risk when writing `H` or `F`.
- Persist decisive evidence in `kb/`, and keep raw metrics under `kb/research/data/` when experiments produce them.

## Anti-Patterns

Do not:

- optimize only for a small eval while claiming mission progress
- benchmark-hack with prompt glue, aliases, or hidden domain mappings presented as intelligence
- use strawman baselines or unfair comparisons
- change too many important variables at once and then claim causality
- treat one lucky run as decisive evidence
- reject a method after an invalid setup
- leave decisive conclusions only in chat

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

## Communication

The active session is the default transport for updates.

- Use concise milestone summaries.
- Ask direct questions when blocked.
- Persist anything that must survive context loss.

Implementation work can happen, but it is not a parallel core artifact graph in Limina. Research drives the contract; delivery details belong to the local project, not the shared template.
