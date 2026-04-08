# Output Template and Scoring Rubric

Use this template when the user has not requested a different structure.

## 1. Research objective
- What exact technical decision or bottleneck are we investigating?
- What would count as a meaningful improvement?

## 2. Assumptions and constraints
- What assumptions am I making because the prompt is underspecified?
- Which constraints materially affect the solution space?

## 3. Challenge map
For each problem slice:
- Problem slice
- Symptom
- Suspected root causes
- Why current systems fail
- Constraints that matter
- What type of mechanism may help

## 4. Search strategy
- Query families used
- Adjacent fields explored
- What evidence I looked for to disconfirm promising ideas
- Which source tiers were used

## 5. Mechanism landscape
Cluster findings by mechanism.
For each cluster include:
- What bottleneck it targets
- How the mechanism works at a high level
- Why it could generalize
- Where it usually fails
- Cost profile
- Maturity level

## 6. Evidence summary
For each serious candidate or cluster include:
- Representative sources
- Strength of evidence
- Evaluation realism
- Reproducibility signals
- Generality
- Composability
- Main limitations

## 7. Top recommendations
### Now
Low-regret actions worth trying soon.

### Next
Promising ideas that need more context or engineering.

### Explore
Frontier or speculative ideas worth watching or testing lightly.

### Avoid / Deprioritize
Ideas that are brittle, weakly supported, too costly, or mismatched.

For every recommendation include:
- Expected upside
- Bottleneck addressed
- Why it matters
- What could invalidate it
- Cheapest validation experiment

## 8. Generalizable insights
Extract lessons that survive beyond the specific papers.

## 9. Open questions
List the unknowns that prevent a stronger recommendation.

## 10. Suggested next searches or experiments
Give the next 3-7 concrete research or validation moves.

## 11. Persistent updates in Limina
- Which `L` notes were created or updated?
- Did the search open or revise any `H`, `CR`, or `SR` artifacts?
- How should `kb/ACTIVE.md` change?

---

# Resource Card

Use this card for each serious paper, benchmark, artifact, or review.

- Title:
- Year:
- Venue / source:
- Resource type:
- Problem slice addressed:
- Mechanism class:
- Core idea in one sentence:
- Why it matters:
- Evidence strength:
- Evaluation realism:
- Main benchmarks / datasets:
- Reported gains:
- Strength of baselines:
- Reproducibility signals:
- Cost to implement:
- Cost to reproduce:
- Cost to operate:
- Generality / transferability:
- Composability:
- Main assumptions:
- Main limitations / risks:
- Recommendation:
- Limina note:

---

# Candidate Scoring Rubric (0-5)

## Problem fit
0 = irrelevant to the real bottleneck
5 = directly addresses a critical bottleneck

## Mechanism generality
0 = one-off or highly domain-bound trick
5 = reusable across tasks / datasets / domains

## Evidence quality
0 = weak or unclear evidence
5 = strong experiments, strong controls, meaningful effect size

## Evaluation realism
0 = toy or badly matched evaluation
5 = realistic benchmarks, strong baselines, sensible metrics

## Reproducibility
0 = no code / unclear method / hard to reproduce
5 = strong code / artifacts / clarity / standardized evaluation

## Implementation feasibility
0 = prohibitively hard to try
5 = cheap and straightforward to prototype

## Operating cost
0 = very expensive or operationally risky
5 = low-cost and production-friendly

## Strategic upside
0 = unlikely to matter materially
5 = could unlock a major improvement or durable capability

## Composability
0 = likely redundant or interfering
5 = likely orthogonal and stackable

## Brittleness risk
0 = extremely brittle
5 = robust across changing contexts

When evidence is incomplete, score with an uncertainty note rather than pretending confidence.
