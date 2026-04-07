# Hypothesis Rubric

## What a strong `H` must do

A good hypothesis is not a topic. It is a decision-relevant claim.

Require all of these:
- name the intervention
- name the comparator baseline
- name the population, task, or eval slice
- name the primary metric
- state the minimum effect size that matters
- state guardrails that must not regress
- state the mechanism that explains why the intervention should work
- state what would make the test invalid
- state confirm, reject, and inconclusive rules

## Quick checklist

Mark the hypothesis incomplete if any box is unchecked.

- [ ] A future decision would actually change based on this result.
- [ ] The comparator baseline is named.
- [ ] The target dataset or eval slice is named.
- [ ] The primary metric and threshold are explicit.
- [ ] The effect size is practically meaningful, not just non-zero.
- [ ] The mechanism is plausible and testable.
- [ ] Method-specific prerequisites are documented.
- [ ] Guardrails are explicit.
- [ ] Failure can be separated from setup error.
- [ ] The result can end as `CONFIRMED`, `REJECTED`, or `INCONCLUSIVE`.

## Strong hypothesis pattern

Use this structure:

> If we apply [intervention] to [population/task], then [primary metric] will move from [baseline] to at least [target] without violating [guardrails], because [mechanism]. We will test this on [dataset/slice] under [method-valid setup]. We will reject only if the candidate fails the threshold after [N trials/seeds] and sanity checks confirm the implementation exposed the method's intended capability.

## Example: weak vs strong

Weak:
- "ColBERT is useful for our retrieval problem."

Strong:
- "If we replace the incumbent single-vector retriever with a late-interaction retriever that uses token-level scoring on the held-out catalog search set, nDCG@10 will improve by at least 5% without p95 latency exceeding 300 ms, because token-level matching should recover exact-attribute relevance that the incumbent misses."

Why the second is better:
- It names the comparator.
- It names the metric and threshold.
- It includes a guardrail.
- It tells you what capability must be exposed in the test.
- It can be confirmed, rejected, or marked inconclusive.

## Method-validity questions

Answer these before finalizing the hypothesis:
1. What exact capability is the method supposed to add?
2. What setup is required to expose that capability?
3. Are we using the method the way its official paper, repo, or docs intend?
4. Could a negative result be explained by the setup instead of the method?
5. Does the baseline get a fair comparison?

If these are not answered, the hypothesis is not ready.

## Common failure modes

Do not accept hypotheses that:
- describe a topic instead of a claim
- omit the comparator
- omit the threshold
- omit the mechanism
- bundle many interventions into one claim
- forget guardrails
- rely on hidden assumptions about data quality or evaluator behavior
- test a candidate in a setup that strips away its core advantage

## What to write in each section of `templates/hypothesis.md`

### Statement
Write the one-sentence claim with intervention, comparator, metric, threshold, and mechanism.

### Mechanism
Explain what capability or failure mode should change, and why.

### Why This Might Generalize
Explain why the claim should hold beyond the current slice, wording, or client-specific data.

### Shortcut Risks
State what could make the result look good without improving the real capability.

### Test Plan
List:
- experiment ID(s)
- dataset or eval slice
- primary metric
- guardrails
- controlled variables
- trial count or seeds
- success, reject, and inconclusive rules
- stop rule or budget

### Evidence
Add both supporting and contradicting sources. Contradictory evidence is often what keeps the experiment honest.

### Conclusion
Use one of:
- `CONFIRMED`
- `REJECTED`
- `INCONCLUSIVE`

Reserve `REJECTED` for method-valid tests only.
