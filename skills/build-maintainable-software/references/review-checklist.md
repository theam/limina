# Review Checklist

Use this reference before finalizing implementation, during refactors, or when asked to review code quality.

## 1. Clarity and naming

- Can a new team member understand the intent from names and structure alone?
- Do functions, modules, and types have one dominant responsibility?
- Are generic names (`manager`, `helper`, `process`, `data`, `misc`, `util`) hiding a real concept?
- Are comments explaining why or invariants instead of repeating the code?

## 2. Boundaries and cohesion

- Is each behavior located in the most obvious place?
- Are side effects easy to find?
- Does each module expose a small, coherent public surface?
- Are infrastructure concerns leaking into domain logic more than necessary?
- Did the change introduce coupling that will make future changes harder?

## 3. Control flow and state

- Is the happy path easy to trace?
- Is branching simpler than before, not more tangled?
- Are invalid states prevented or at least checked close to the boundary?
- Is mutable state owned by one place, with explicit transitions?

## 4. API and error handling

- Are inputs validated at the correct boundary?
- Are errors explicit, actionable, and consistent?
- Are edge cases handled deliberately rather than by accident?
- Would another engineer know how to use this API correctly without reading its implementation?

## 5. Tests and safety

- Do tests cover the changed behavior and the main regression risk?
- Are tests written at the smallest level that still protects behavior?
- Can the implementation be refactored without rewriting half the test suite?
- If the change is risky, is observability or logging sufficient to diagnose failures?

## 6. Scope control

- Did the implementation solve the requested problem without unnecessary architecture?
- Was any abstraction added for present needs rather than hypothetical reuse?
- Could code, state, branches, or configuration be removed instead of added?
- Is the change incremental enough to review and rollback safely?

## Severity guide for reviews

### High severity

Use for problems that risk correctness, data integrity, security, concurrency safety, or a public contract that is likely to cause misuse.

Examples:
- Hidden side effects that can break invariants
- Missing validation at trust boundaries
- Concurrency hazards or non-idempotent retry paths
- Ambiguous public APIs that invite incorrect usage

### Medium severity

Use for maintainability issues that will slow future work or make bugs more likely.

Examples:
- Mixed responsibilities
- Indirection without payoff
- State ownership that is hard to reason about
- Tests coupled to internal implementation details

### Low severity

Use for local cleanup or polish that does not materially change correctness or future change cost.

Examples:
- Small naming improvements
- Minor duplication with one obvious cleanup path
- Comment wording or formatting

## Minimum bar before considering the task done

- Intent is obvious from structure and names.
- Side effects and boundaries are visible.
- The chosen abstraction level matches the actual problem.
- Tests protect the changed behavior.
- The change is no larger than necessary.
