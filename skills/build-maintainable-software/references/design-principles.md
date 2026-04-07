# Design Principles

Use this reference for implementation planning, architecture changes, and non-trivial refactors.

## 1. Model the domain before the implementation

- Start from the business concept, not the framework primitive.
- Choose names from the problem domain and keep one concept mapped to one term.
- Separate decision logic from orchestration. A function should either decide, transform, or coordinate, but not all three unless the scope is truly tiny.
- Prefer data shapes that mirror how the problem is understood by the team.

## 2. Define boundaries early

- Make module ownership explicit: who owns this state, this rule, this side effect, and this public contract?
- Keep dependency direction stable. Core logic should not depend on volatile delivery details unless the project intentionally centers on that framework.
- Avoid cycles across modules, packages, or services.
- Pass capabilities explicitly instead of reaching through globals or service locators.

## 3. Introduce abstractions late and for a reason

- Add an abstraction when it removes real complexity, hides a stable seam, or supports multiple concrete cases that already exist.
- Avoid creating interfaces, base classes, or generic helpers "just in case."
- Prefer concrete code until multiple uses reveal the right abstraction.
- If an abstraction has to explain itself with a long comment, it is probably too indirect.

## 4. Keep data and state simple

- Minimize mutable shared state. Prefer a clear owner and explicit transitions.
- Make state transitions visible in one place when possible.
- Avoid boolean parameter flags for behavior switches. Split functions or use a named value/object when behaviors are meaningfully different.
- Prefer immutable inputs and outputs for transformation logic.

## 5. Design APIs to be easy to use correctly

- Keep public surfaces small and intention-revealing.
- Validate at boundaries and fail loudly with actionable errors.
- Avoid sentinel values and ambiguous null-like returns when a richer result or error object is clearer.
- Make success and failure paths symmetric and predictable.
- Hide irrelevant implementation choices behind task-focused operations.

## 6. Structure for reading order

- Organize files and modules so a reader can move from entry point to core logic without jumping through many layers.
- Keep related behavior together, even if that means accepting a little duplication.
- Prefer short indirection chains. Every extra hop should buy clarity or isolation.
- Group helper logic near the behavior it supports unless it is broadly useful and conceptually stable.

## 7. Test behavior, not wiring trivia

- Test the most valuable rules, invariants, and regression risks first.
- Prefer tests that verify externally visible behavior over tests that mirror implementation details.
- Use unit tests for deterministic logic, integration tests for boundaries, and end-to-end tests for a small set of critical flows.
- Remove or rewrite brittle tests that block safe refactoring without protecting real behavior.

## 8. Treat scalability as a design property, not an excuse for premature complexity

- Choose designs that make bottlenecks observable and replaceable.
- Measure before optimizing.
- Add caching, batching, concurrency, retries, or pooling when constraints justify them, and make the trade-offs explicit.
- When introducing concurrency, define ownership, cancellation, timeout, retry, idempotency, and backpressure rules.

## 9. Refactor with small, reversible moves

- Rename first when names are wrong.
- Separate pure logic from side effects.
- Split long functions into orchestration plus focused helpers.
- Collapse layers that add no protection or meaning.
- Delete dead code, unused parameters, and stale branches aggressively once safety is established.

## Common smell -> better move patterns

- Long function with mixed concerns -> split into orchestration + pure helpers + edge adapters.
- Generic `utils` bucket -> move helpers closer to the owning module or create a focused domain service.
- Boolean mode parameters -> separate functions, strategies, or a named command object.
- Repeated conditionals scattered across files -> centralize policy or model the variation explicitly.
- Shared mutable state touched from many places -> give the state one owner and expose narrow operations.
- Leaky infrastructure details in business logic -> wrap them in an adapter and keep business rules framework-light.
