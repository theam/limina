# Limina — Codex Runtime Adapter

This file is the **Codex runtime adapter** for the shared Limina contract.

`README.md` is the canonical human-readable specification. `CLAUDE.md` and `AGENTS.md` must stay **functionally equivalent** while using runtime-specific tools and wording.

## Non-Negotiable Rules

1. Everything durable goes in `kb/`. If it only exists in chat, it is not persistent state.
2. Each unit of work is a task tracked through `kb/mission/BACKLOG.md`.
3. Research work follows `H -> E -> F`. Do not run experiments before the hypothesis exists.
4. Engineering work follows `INV -> FT -> IMP -> RET`.
5. `CR` and `SR` are first-class review artifacts. Use them, do not collapse them into generic notes.
6. Communicate milestones, blockers, and handoffs in the active session; do not assume an external notification channel.
7. Run `python3 scripts/kb_validate.py` after substantial KB edits and before closing KB-heavy work.
8. Synthesize before closing a mission. When all tasks are DONE, distill reusable knowledge into Knowledge Cards (`templates/knowledge-card.md`). Write them to `shared-knowledge/cards/` if available, else `kb/cards/`.

## Runtime Mapping

Use Codex-native mechanisms to satisfy the shared contract:

- Ask the user with `request_user_input` when structured options help, or with a direct concise question when they do not.
- Delegate bounded work with `spawn_agent` and coordinate with `send_input` / `wait`.
- Keep durable progress in `kb/`, not only in agent messages.
- Treat `README.md` as the conceptual source of truth and this file as operational guidance.

## Working Protocol

At the start of a new mission (first session only):

1. If `shared-knowledge/` exists, read `shared-knowledge/INDEX.md`
2. Grep `shared-knowledge/cards/` for terms related to the challenge
3. Read relevant Knowledge Cards and note applicable insights

At the start of every session:

1. Read `kb/INDEX.md`
2. Read `kb/mission/BACKLOG.md`
3. Read the active task file if one exists
4. Read the relevant local docs before acting

During work:

- Update the current artifact's `Progress` section at stopping points
- Keep `INDEX.md` and `BACKLOG.md` in sync with core artifact creation and completion
- Write reusable lessons to the appropriate persistent location, not only to chat
- Search `kb/` before creating new artifacts to avoid duplicates and contradictions

Before closing KB-heavy work:

1. Ensure task and artifact state is written to `kb/`
2. Run `python3 scripts/kb_validate.py`
3. Fix KB errors before treating the work as complete
4. Post a concise handoff summary in the active session

Before closing a mission (all tasks DONE):

1. Review findings (HIGH/MEDIUM impact), decisions, and lessons learned
2. Generate Knowledge Cards for reusable insights using `templates/knowledge-card.md`
3. Write cards to `shared-knowledge/cards/` (or `kb/cards/` if no shared dir)
4. Update `shared-knowledge/INDEX.md` with new card entries
5. Write a mission summary to `shared-knowledge/missions/`

## Artifact Model

The validator-enforced core artifacts are:

- `T`, `H`, `E`, `F`, `L`
- `FT`, `INV`, `IMP`, `RET`
- `CR`, `SR`

`DECISIONS.md` and `CEO_REQUESTS.md` are mission ledgers. Optional milestone notes are allowed, but they are not part of the enforced core graph.

## Communication

The active session is the default transport for updates.

- Use concise progress summaries for milestones and handoffs
- Ask the user early when blocked
- Persist anything that must survive context loss in `kb/`
