# Limina — cook instructions

This repository uses the slim Limina contract.

Progress lives in `kb/`, but the always-on state is intentionally small:

- `kb/mission/CHALLENGE.md`
- `kb/ACTIVE.md`
- linked research artifacts

## Continuity protocol

Each iteration starts with recovery, not action.

Before doing anything:

1. Read your runtime adapter.
2. Read `kb/mission/CHALLENGE.md`.
3. Read `kb/ACTIVE.md`.
4. Open only the linked artifacts you need next.

Then decide:

- What is the current objective?
- What is the highest-signal next step?
- What assumption or trust issue could invalidate the current direction?

## Non-negotiable rules

- Everything durable goes in `kb/`.
- Research follows `H -> E -> F`.
- Keep files small and scoped.
- Ask the user directly when blocked.

## When blocked

1. Write the blocker in `kb/ACTIVE.md`.
2. Persist any evidence gathered so far in the active artifact.
3. Stop and ask for human input.
