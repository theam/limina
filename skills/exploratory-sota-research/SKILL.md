---
name: exploratory-sota-research
description: Map the AI/ML state of the art for a concrete technical problem. Use for exploratory research, mechanism-based literature search, benchmark mapping, and finding generalizable approaches. Not for simple fact lookup.
---

# Exploratory SOTA Research

Use this skill to map the external mechanism landscape for a hard AI/ML research problem and turn that map into concrete Limina artifacts.

## When to use it

Use this skill when the user wants any of the following:
- an exploratory literature search
- a state-of-the-art map
- a mechanism landscape for a technical problem
- candidate research directions for AI/ML/DL/LLM systems
- promising approaches to improve a bottleneck
- a benchmark, reproducibility, or artifact scan
- a recommendation about which technical directions are worth testing

Do not use this skill for:
- simple factual lookup
- single-paper summarization when no broader research mapping is needed
- generic brainstorming with no research component
- implementation-only requests
- questions where the user clearly wants a direct answer rather than an exploratory research process

## Limina adapter

When you use this skill inside a Limina repo:

1. Read `kb/mission/CHALLENGE.md`.
2. Read `kb/ACTIVE.md`.
3. Open only the linked artifacts you need next.
4. Persist serious external sources as `L` notes in `kb/research/literature/`.
5. If the landscape changes the direction, update or create the relevant `H`, `CR`, `SR`, and `ACTIVE.md` entries instead of leaving the conclusion only in chat.
6. Prefer `python3 scripts/kb_new_artifact.py ...` when a new core artifact is needed.

The goal is not just to produce a good survey. The goal is to change the research graph in a grounded way.

## Non-negotiable principles

1. Solve the real decision problem.
   Infer the objective, constraints, and decision context behind the request.

2. Search by challenge and mechanism.
   Convert the task into problem slices, failure modes, root causes, and mechanism families.

3. Prefer generalizable solutions.
   Prioritize mechanisms that transfer across datasets, tasks, or domains unless the user explicitly wants a narrow patch.

4. Evidence beats hype.
   Reward realistic baselines, meaningful gains, strong ablations, transparent limitations, reproducibility signals, and public artifacts.

5. Seek disconfirming evidence.
   Actively search for failures, weaknesses, benchmark mismatch, and hidden assumptions.

6. Do not return a flat paper list.
   Always synthesize by bottleneck and mechanism, then recommend actions.

## Workflow

### 1. Frame the decision problem

Before searching, define:
- the real objective
- success criteria or metrics
- relevant constraints
- the main problem slices

Typical constraints include:
- latency
- compute budget
- training budget
- inference cost
- data regime
- supervision availability
- privacy or licensing
- reliability
- multilinguality
- deployment complexity
- domain shift
- online vs offline use

If the request is underspecified, proceed with explicit assumptions rather than stalling.

### 2. Build a challenge map

For each problem slice, identify:
- symptom
- suspected root causes
- why current systems fail
- what constraints matter
- what type of intervention might help

Use root-cause language instead of surface-level language.

Bad framing:
- "retrieval is weak"

Better framing:
- "candidate-generation embeddings fail to preserve the distinctions needed for this query class under domain shift and sparse supervision"

### 3. Generate mechanism families

For each challenge, derive candidate mechanism families.
Search both direct and indirect solutions.

For every challenge, think in four buckets:
- direct mechanisms
- indirect mechanisms
- cross-domain analogies
- evaluation mechanisms

### 4. Create the search agenda

Generate multiple query families, not one query.
For each important slice, search across:
- direct task wording
- mechanism wording
- benchmark and evaluation wording
- limitation and failure wording
- systems and cost wording
- adjacent-field wording
- seminal and frontier wording

Always search for both:
- supporting evidence
- and reasons the mechanism may not generalize

### 5. Search in expanding rings

Search in this order unless the problem clearly requires something else:
1. canonical high-signal sources
2. discovery and citation infrastructure
3. benchmarks, artifacts, and official code
4. peripheral but plausible sources
5. weak-signal sources only for lead generation

A fuller source policy lives in `references/source-selection.md`.

### 6. Read strategically

Use layered reading rather than uniform reading.

Pass 1: triage
- title
- abstract
- intro
- figures
- experiments overview
- conclusion and limitations

Pass 2: credibility check
- method
- setup
- baselines
- ablations
- robustness checks
- failure cases

Pass 3: transferability check
- assumptions
- data regime
- compute demands
- domain dependence
- hidden tuning
- reusability

Pass 4: integration check
- where it fits in a real system
- interfaces that would change
- operational risks
- likely interactions with current components

### 7. Persist serious candidates

For every serious candidate or source cluster, record at minimum:
- title
- year
- venue or source
- problem slice
- mechanism class
- one-sentence idea
- evidence strength
- evaluation realism
- reproducibility signals
- cost profile
- generality
- composability
- limitations
- recommendation

Inside Limina:
- create or update `L` notes for serious sources, not just ad-hoc notes in chat
- update `ACTIVE.md` when the working set changes
- if the search materially changes the research direction, create or update `H`, `CR`, or `SR`

Use the fuller output shape in `references/output-template.md`.

### 8. Synthesize by mechanism

Do not return a chronological list of papers.
Cluster findings by:
- bottleneck addressed
- mechanism family
- evidence quality
- cost profile
- generality
- likely durability

Always separate:
- reusable mechanisms
- narrow hacks
- mature evidence
- frontier but fragile ideas
- ideas that are promising only in combination

### 9. Recommend an action portfolio

Always conclude with:
- **Now** — low-regret, high-value next moves
- **Next** — promising but more context-dependent
- **Explore** — frontier or uncertain ideas worth monitoring
- **Avoid / Deprioritize** — weak, brittle, or mismatched directions

For each recommendation, state:
- expected upside
- bottleneck addressed
- why it matters
- what could invalidate it
- the cheapest useful validation experiment

## Default answer structure

Unless the user asked for a different format, return:
1. Research objective
2. Assumptions and constraints
3. Challenge map
4. Search strategy
5. Mechanism landscape
6. Evidence summary
7. Top recommendations (`Now / Next / Explore / Avoid`)
8. Generalizable insights
9. Open questions
10. Suggested next searches or experiments
11. Persistent updates in Limina

## Output quality bar

A good answer:
- is broad but selective
- is skeptical but constructive
- makes trade-offs explicit
- distinguishes general solutions from narrow ones
- explains why a mechanism matters
- gives the user a practical next move

A poor answer:
- mirrors the task label without decomposition
- returns a flat bibliography
- treats citations or venue prestige as enough
- ignores cost and reproducibility
- recommends mechanisms without saying where they fail
- leaves decisive conclusions only in chat when Limina artifacts should change

## Additional resources

Load these only when helpful:
- `references/source-selection.md` — source tiers, what each source is for, and filtering rules
- `references/output-template.md` — full report template, scoring rubric, and resource card format
- `references/worked-example-information-retrieval.md` — worked example for retrieval, search, filtering, and ranking problems
- `evals/trigger-prompts.csv` — positive and negative prompts for trigger testing and regression checks

## Final instruction

Be an exploratory researcher, not a keyword search engine. Inside Limina, leave behind durable notes and next moves, not just a polished survey.
