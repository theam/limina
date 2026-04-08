---
name: experiment-rigor
description: Design, review, and conclude rigorous research experiments for Limina's H→E→F workflow. Use when creating or revising hypotheses, experiments, or findings; defining baselines, datasets, metrics, or stopping rules; comparing candidate methods; deciding whether a result is conclusive; or when a negative result may be caused by an invalid setup rather than a true method failure.
---

# Experiment Rigor

Use this skill to turn Limina research into decision-grade evidence.

## When to use it

Use this skill when you need to:
- write or revise a serious `H`, `E`, or `F`
- define a fair comparator baseline
- decide whether a result is conclusive, inconclusive, or invalid
- choose metrics, guardrails, slices, seeds, or stopping rules
- review whether a negative result reflects the method or only the setup

Do not use it for:
- generic brainstorming with no concrete research decision
- one-off coding tasks with no experiment design component
- literature mapping when the main gap is external landscape search rather than experiment validity

## Read first

Read:
- `kb/mission/CHALLENGE.md`
- `kb/ACTIVE.md`
- only the linked `H`, `E`, `F`, `L`, `CR`, or `SR` notes relevant to the current question

Before evaluating any nontrivial method, read the official paper, repo, or docs for that method. Record the setup requirements needed to expose its claimed advantage.

Use the reference files on demand:
- `references/hypothesis-rubric.md` — writing or revising `H`
- `references/experiment-rubric.md` — designing or reviewing `E`
- `references/metrics-storage.md` — storing raw metrics and lineage under `kb/research/data/`

## Non-negotiable rules

1. Optimize for decisive evidence. If an experiment cannot change a decision, redesign it.
2. Protect method validity. Never test a method in a setup that strips away the capability you are trying to evaluate.
3. Compare fairly. Use strong baselines and control non-essential variables.
4. Separate invalid test, implementation failure, insufficient signal, and true negative result.
5. Reject a hypothesis only after a method-valid test with enough signal. Otherwise mark it inconclusive and design the next experiment.
6. Keep `kb/` as canonical memory. Narrative belongs in `H/E/F`; raw metrics belong in structured files under `kb/research/data/`.
7. Use the current Limina templates and keep `## Links` valid. Prefer `python3 scripts/kb_new_artifact.py ...` when a new core artifact is needed.
8. Use `CR` or `SR` only when the direction is blocked, invalidated, plateaued, or strategically changing. Do not trigger them on a fixed cadence.

## Workflow

### 1. Start from the decision

Write the decision in one sentence before writing the hypothesis or experiment.

Examples:
- "Adopt candidate retriever A only if it beats the incumbent on nDCG@10 by at least 5% without violating latency."
- "Keep the current agent scaffold unless the new scaffold improves task success by a practically meaningful margin."
- "Attribute the regression to chunking, not model choice, only if the chunking ablation reproduces most of the drop."

If the experiment will not resolve a specific decision, do not run it yet.

### 2. Strengthen the hypothesis

Write `H` as a falsifiable, thresholded claim with:
- an intervention
- a named comparator
- a target slice or population
- a primary metric
- guardrails
- a mechanism that explains why the intervention should work

Use this shape:

> If we apply [intervention] to [population/task], then [primary metric] will move from [baseline] to at least [target] without violating [guardrails], because [mechanism]. We will test this on [dataset/slice] under [method-valid setup]. We will reject only if the candidate fails the threshold after [N trials/seeds] and sanity checks confirm the implementation exposed the method's intended capability.

Fill the current hypothesis template like this:
- `Statement`: intervention, comparator, slice, primary metric, threshold, and mechanism
- `Mechanism`: the exact capability or failure mode you expect to change
- `Why This Might Generalize`: why the claim should survive beyond the current eval slice
- `Shortcut Risks`: what could make the result look good without improving the real capability
- `Test Plan`: baseline, metrics, guardrails, controlled variables, seeds/trials, stop rule, and confirm/reject/inconclusive criteria
- `Evidence`: supporting and contradicting sources or prior artifacts
- `Conclusion`: use `CONFIRMED`, `REJECTED`, or `INCONCLUSIVE`

### 3. Run the method-validity gate

Answer these questions explicitly before finalizing `H` or `E`:
1. What exact capability or mechanism is the method supposed to add?
2. What scoring rule, prompt pattern, data shaping, indexing strategy, training recipe, or tool access is required to expose that capability?
3. Are we using the method as its official paper, repo, or docs intend?
4. Are we accidentally stripping away its core mechanism?
5. Does the baseline get a fair comparison?
6. Could failure be explained by implementation, preprocessing, evaluation, or dataset mismatch rather than the method itself?

Do not proceed until the test is method-valid.

### 4. Design a decisive experiment

Design `E` to answer one important question with one primary metric.

In the current experiment template:
- `Objective`: state the decision question and the adoption/rejection threshold
- `Setup`: record environment, code revision, dataset version/split, model and prompt versions, hardware, external services, and any deliberate changes from baseline
- `Procedure`: use exact commands or deterministic steps
- `Expected Outcome`: write confirm, reject, and inconclusive conditions before execution
- `Progress`: update at every stopping point
- `Results`: summarize top-line metrics and link the raw metric directory under `kb/research/data/E###/`
- `Analysis`: compare against baseline, quantify uncertainty when possible, and explain failure modes or validity threats
- `Decision`: state the next action immediately

Require every meaningful experiment to include:
- a named comparator baseline
- a primary metric
- guardrail metrics
- an eval dataset or named slices
- a trial plan for stochastic systems
- a stop rule or budget ceiling
- a raw metric storage path under `kb/research/data/`

### 5. Store raw metrics properly

Keep the narrative in `H/E/F`. Keep raw data in machine-readable files under `kb/research/data/`.

Minimum requirement for a completed experiment:
- `manifest.json`
- `summary.json`
- at least one per-run file such as `runs.csv` or `runs.jsonl`

If MLflow or W&B exists, log there too, then mirror the decisive summary and run IDs back into `kb/`.

### 6. Interpret negative results carefully

Before concluding "the method does not work", classify the result:
- `valid negative`: method-valid test failed to beat the threshold
- `invalid test`: the setup did not expose the claimed capability
- `implementation failure`: bug, bad preprocessing, wrong configuration, or tool problem
- `insufficient signal`: too few examples or trials, or too much variance
- `trade-off failure`: quality improved but violated guardrails

Only the first class supports `REJECTED`.
The other classes require a fix, a redesigned experiment, or an explicit `INCONCLUSIVE` conclusion.

### 7. Write a decision-grade finding

Write `F` as a decision-grade statement, not a lab notebook dump.

In the current finding template:
- `Finding`: state the direction and magnitude in one sentence
- `Evidence`: cite the experiment, dataset or slice, trial count, and uncertainty if available
- `What Improved For Real`: explain what capability changed beyond the benchmark number itself
- `Remaining Debt`: note what is still a workaround, shortcut, or open risk
- `Next Move`: create the next hypothesis, review, or concrete follow-up immediately

Good:
- "Candidate B improved task success from 62% to 71% on the held-out workflow set across 5 trials while keeping median latency within the budget."

Bad:
- "We tried Candidate B and it felt better on several cases."

### 8. Escalate only when the direction actually changes

Use `CR` or `SR` when:
- the result invalidates the current framing
- the direction has plateaued and needs broader review
- the eval, baseline, or assumptions can no longer be trusted
- the research should narrow, reset, or pivot

When you create or update those review notes, also update `kb/ACTIVE.md` and keep the parent/child `## Links` graph consistent.

## Refuse these anti-patterns

Do not:
- reject a method after testing it in a setup that removes its key mechanism
- change model, data, prompt, preprocessing, and metric all at once without isolating cause
- report only absolute scores when the decision depends on deltas vs baseline
- rely on one lucky run
- use only generic academic metrics when the mission needs task-specific success criteria
- let raw results live only in chat or screenshots
- call an experiment "failed" without saying whether the failure was methodological, implementation, or real
