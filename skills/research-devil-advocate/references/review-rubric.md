# Review Rubric

Use this rubric to pressure-test the current direction. You do **not** need to answer every question mechanically. Focus on the questions that most affect the decision.

## 1. Problem framing and success criteria
- Is the mission defined as a real decision problem or just a vague curiosity?
- Are metrics aligned with mission value or are we optimizing a proxy?
- What threshold would count as “good enough” to continue or ship?
- If results improve metric X but worsen cost, latency, complexity, or robustness, has that trade-off been acknowledged?

## 2. Hypothesis quality
- Is the hypothesis falsifiable?
- What observation would disprove it?
- Is the expected mechanism clear, or are we just hoping?
- Are we testing a causal claim with only correlational evidence?

## 3. Baselines and controls
- What is the honest baseline?
- Is it tuned fairly and given comparable budget, data, and evaluation conditions?
- Are there negative controls or ablations?
- Could the proposed method look good only because the baseline is weak?

## 4. Experimental validity
- Are data splits, seeds, and evaluation procedures sound?
- Is there leakage, contamination, or hindsight bias?
- Are we measuring once when replication is needed?
- Are sample sizes large enough to separate signal from noise?

## 5. Evidence strength
- Does the reported effect exceed noise and operational variance?
- Are we looking at effect size, not just direction?
- Is the conclusion narrower than the data, or broader?
- Would a skeptical external reviewer accept this as evidence?

## 6. Alternative explanations and confounders
- What else could explain the result?
- Did any uncontrolled variable change with the intervention?
- Could implementation mistakes or instrumentation artifacts produce the same pattern?
- Are we mistaking correlation for causation?

## 7. Search breadth and local maxima
- Are we iterating on one family of ideas because it is familiar?
- What 3 genuinely different approaches remain untested?
- If starting from scratch today, would we still choose this path?
- Are we close to the ceiling of this approach?

## 8. Opportunity cost and value of information
- Does the next experiment meaningfully change the decision?
- Are we spending days to learn something we already mostly know?
- Is there a cheaper discriminating test?
- What work should be paused or killed right now?

## 9. Decision hygiene
- Are we escalating commitment because of sunk cost?
- Are we rewriting the narrative to fit the latest result?
- Have success / pivot / stop criteria been declared in advance?
- Does the recommended next step match the actual uncertainty?

## 10. KB integrity and reproducibility
- Are `CHALLENGE`, `ACTIVE`, and the relevant `H`, `E`, `F`, `L`, `CR`, or `SR` artifacts linked coherently?
- Is the reasoning reproducible from files, not just chat memory?
- Are contradictory artifacts called out?
- Would another researcher be able to reconstruct the case?

## Common bias patterns to call out explicitly
- confirmation bias
- cherry-picking
- narrative fallacy
- sunk-cost bias
- local-maximum bias
- metric gaming
- survivorship bias
- authority bias
- premature convergence

## Severity guide
- **CRITICAL**: likely invalidates the conclusion or wastes substantial time if ignored
- **HIGH**: materially weakens the decision or creates substantial risk
- **MEDIUM**: real issue, but not a thesis-breaker
- **LOW**: worthwhile improvement, limited decision impact
