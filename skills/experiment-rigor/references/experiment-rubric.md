# Experiment Rubric

## Preflight

Do not run the experiment until all of these are true:
- [ ] One decision question is written.
- [ ] One primary metric is written.
- [ ] A comparator baseline is named.
- [ ] The method-validity gate is passed.
- [ ] Adoption, rejection, and inconclusive conditions are written.
- [ ] A budget or stopping rule is written.
- [ ] A raw metric storage path is chosen under `kb/research/data/`.

## Baselines and controls

Use baselines intentionally:
- current production or incumbent baseline
- simple/reference baseline
- strong in-family baseline when comparing families

Control everything that is not the deliberate intervention:
- dataset or eval slice
- prompt framing
- preprocessing
- postprocessing
- scoring rules
- latency budget
- tool access
- hardware class, if relevant

If multiple changes are unavoidable, split the work:
1. Run an enabling experiment to make the candidate viable.
2. Run a fair head-to-head comparison.
3. Run ablations to identify what mattered.

## Data and eval slices

Define the data before execution:
- main evaluation set
- important edge-case slices
- exclusions
- contamination risks
- how labels or graders were created
- whether examples are synthetic, human-labeled, or production-derived

Prefer held-out data for decision-making.
If you must create synthetic data, label it clearly and keep it separate from real-world slices.

## Metrics

For each experiment, define:
- one primary metric that decides adoption
- secondary metrics that explain trade-offs
- guardrails that prevent unacceptable regressions

Examples of guardrails:
- latency
- cost
- recall floor
- safety violation rate
- calibration drift
- hallucination rate
- system stability

Always record:
- absolute value
- delta vs baseline
- units
- slice name
- trial count

## Grading strategy

Choose the simplest grader that is reliable enough.

Order of preference:
1. Code-based grading
2. Human grading
3. LLM-based grading

For LLM-based graders:
- write clear rubrics
- calibrate on a human-reviewed slice first
- grade dimensions separately when possible
- keep grader prompts versioned

For agentic systems:
- prefer outcome or environment-state grading over brittle path enforcement
- use transcript analysis as secondary diagnostics unless process compliance is the goal

## Trials, seeds, and uncertainty

Treat stochastic outputs as stochastic evidence.

Require:
- multiple seeds or trials when output variance matters
- aggregation at the example or task level when possible
- trial count recorded in the experiment and raw files

When feasible, record uncertainty:
- confidence interval
- standard error
- error bars
- bootstrap range
- seed-to-seed spread

Do not treat a single successful run as decisive.

## What to write in each section of `templates/experiment.md`

### Objective
State the decision question and the threshold that matters.

### Setup
Record:
- environment
- code revision
- dataset version and split
- model, prompt, and scorer versions
- hardware and compute
- external services or APIs
- deliberate differences from baseline and why they changed

### Procedure
Use exact commands or deterministic steps.
Anyone should be able to rerun the experiment from this section plus the manifest.

### Expected Outcome
Write three explicit outcomes:
- confirm
- reject
- inconclusive

### Progress
Update at every stopping point.
Use it as the breadcrumb trail for future sessions.

### Results
Link to raw metric files and summarize top-line numbers in the experiment file.
If anything threatened validity, record it here and address it in `Analysis`.

### Analysis
Explain:
- delta vs baseline
- uncertainty
- failure modes
- whether the method-validity gate remained satisfied
- whether follow-up ablations are needed

### Decision
State one of:
- advance this direction
- reject this direction
- run follow-up experiment
- escalate to challenge / strategic review

## Done criteria

Mark an experiment done only when one of these is true:
- the candidate clears the adoption threshold and guardrails
- the candidate fails under a method-valid setup
- the budget is exhausted and the result remains inconclusive
- the direction has plateaued or the framing is no longer trustworthy

## Anti-patterns

Refuse these:
- strawman baselines
- uncontrolled simultaneous changes
- only absolute scores, no baseline deltas
- only aggregate metrics, no slice checks
- screenshots instead of structured raw metrics
- post-hoc threshold changes after seeing the results
- declaring "method failure" when the setup was invalid
