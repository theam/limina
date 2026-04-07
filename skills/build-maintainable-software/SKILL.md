---
name: build-maintainable-software
description: Build, review, refactor, or design software for readability, simplicity, modularity, testability, and safe changeability. Use for feature work, architecture decisions, API boundaries, naming, refactors, and code reviews across any language or stack.
---

# Build Maintainable Software

## Overview

Build code that is easy to read, easy to change, and hard to misuse. Favor clear intent, cohesive modules, explicit boundaries, predictable control flow, and tests that protect behavior without overcoupling to implementation.

Load the smallest set of references that fits the task. Do not load every reference by default.

## When to use it

Use this skill when the work is mainly about:
- implementing or refactoring production code
- reviewing code for correctness, maintainability, or safe changeability
- designing module boundaries, APIs, naming, ownership, or test strategy
- simplifying an over-abstracted or fragile implementation

Do not use it when the main problem is:
- external landscape search or state-of-the-art mapping
- experiment design, baseline selection, or interpreting research evidence
- a request for broad rewrites driven mostly by style preference rather than concrete change value

## Workflow

1. Clarify the behavior change, not just the code change. Identify the user-facing outcome, constraints, invariants, and affected boundaries.
2. Start with the simplest design that makes the intent obvious. Avoid speculative abstractions, indirection, configuration, and generalization.
3. Decide boundaries before coding: ownership of state, direction of dependencies, public API surface, and locations of side effects.
4. Make names reveal intent. Prefer domain terms, stable concepts, and small interfaces over generic utilities and overloaded helpers.
5. Keep execution flow unsurprising. Flatten nesting, reduce control flags, isolate branching, and make data transformations explicit.
6. Keep business rules close together and push framework, I/O, time, randomness, and network code to the edges when practical.
7. Add or update tests at the right level for the change: unit tests for decision logic, integration tests for boundaries and wiring, end-to-end tests only for critical flows.
8. Before finishing, run the final pass in [references/review-checklist.md](references/review-checklist.md).

## Default Operating Assumptions

- Optimize first for clarity, then for changeability, then for performance unless the task provides measured performance constraints.
- Prefer composition over inheritance unless inheritance cleanly models a stable is-a relationship.
- Prefer cohesive modules with narrow interfaces over large shared helpers or god objects.
- Prefer explicit dependencies and data flow over hidden globals, ambient context, and action at a distance.
- Prefer removing code, states, branches, and configuration over introducing abstractions prematurely.
- Keep public APIs small, consistent, and difficult to misuse.
- Make invalid states hard to represent; encode invariants in types, constructors, validation, or module boundaries.
- Follow repository conventions unless they clearly harm correctness or maintainability.

## Decision Rules

When trade-offs are ambiguous, prefer:

1. Clear, slightly repetitive code over clever indirection.
2. Cohesion over aggressive DRY when duplication is small and concepts change for different reasons.
3. One obvious path through the code over highly flexible branching.
4. Stable contracts over convenience-based coupling.
5. Incremental refactors over big-bang rewrites.
6. Measured optimization over guessed micro-optimizations.

## Reference Guide

- Open [references/design-principles.md](references/design-principles.md) for new features, major refactors, architecture changes, module boundaries, API design, naming, state management, error handling, or dependency direction.
- Open [references/review-checklist.md](references/review-checklist.md) for code review, cleanup, simplification, test strategy, risk checks, or the final maintainability pass.
- If the repository has local guidance (`AGENTS.md`, `CLAUDE.md`, `CONTRIBUTING.md`, architecture docs, or test/lint scripts), follow those specifics first and use this skill as the cross-project default.
- In Limina projects, use this skill for implementation, refactor, and review work that supports the mission. Do not use it to replace the research loop or to justify direction changes without evidence.
- In Limina projects, pair this skill with:
  - `$experiment-rigor` when the code change is part of experiment design, execution, or interpretation
  - `$exploratory-sota-research` when the missing piece is external research rather than code quality or design

## Output Expectations

- For implementation work: keep the change small, make the design intent obvious, and verify the behavior with the right level of tests or checks.
- For reviews: prioritize correctness risks first, then maintainability risks, then optional polish. Keep findings concrete and tied to future change cost or bug risk.
- For refactors: preserve behavior unless the requested goal is a behavior change, and make the rollback surface obvious.

## Execution Notes

- Do not rewrite broad areas of the codebase just to impose a preferred pattern. Improve along natural seams.
- When touching legacy code, leave the surrounding area clearer than you found it, but keep the behavioral delta tight.
- When reviewing, distinguish correctness risks, maintainability risks, and optional polish. Prioritize the first two.
- When comments are needed, explain why, invariants, or non-obvious trade-offs. Do not narrate obvious code.
- If the work changes a mission-critical behavior, evaluation surface, or system assumption in Limina, persist the relevant rationale or follow-up evidence in the repository's `kb/` flow.
- If a requested design conflicts with these rules, explain the trade-off briefly and follow the user's explicit intent.
