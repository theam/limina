# Limina

Limina is a research-first template for autonomous technical investigation.

It is intentionally small:

- one shared machine-facing contract
- one active-state file
- one required research flow: `H -> E -> F`
- a small persistent knowledge base in `kb/`
- only deterministic hooks and validation in the runtime

The goal is not to model every kind of work. The goal is to help an agent run clean research with durable evidence and low context noise.

## Core Idea

Limina treats research as the primary workflow.

- `H` hypotheses capture what might work and why.
- `E` experiments capture how it was tested.
- `F` findings capture what the evidence actually says.
- `CR` and `SR` are review artifacts for real decision points, not routine bureaucracy.

Implementation can happen, but it is an outcome of research, not a parallel artifact graph in the core contract.

## Why This Version Is Smaller

The template used to carry:

- task files
- backlogs
- manual last-ID bookkeeping
- an engineering artifact graph
- reminder and nudge hooks
- a growing lessons block inside runtime instructions

That made the prompt surface large and noisy. The slim contract keeps only the parts that create hard guarantees or durable evidence.

## Quick Start

Open Claude Code or Codex and paste:

```text
Install the Limina research skill by running:
curl -fsSL https://raw.githubusercontent.com/theam/limina/main/setup.sh | bash
Then ask me to change my working directory to the folder where I want
my research project to live, and help me set up a new Limina research project.
```

The skill clones the template, asks for the mission, and prepares the initial `kb/`.

To start:

```bash
cd <your-project-name> && claude
```

Or open the folder in Codex.

## Bundled Skills

The template ships with one launcher skill plus repo-local companion skills:

- `limina` — set up and launch a Limina research project
- `experiment-rigor` — design, review, and conclude decision-grade `H -> E -> F` work
- `exploratory-sota-research` — map mechanism landscapes, literature, and reproducibility signals
- `article-strategy` — turn kb evidence into publishable article ideas and drafts
- `notion-sync-kb` — sync `kb/` into Notion

After creating a project from this template, install the bundled skills from inside the repo:

```bash
bash scripts/install_skills.sh
```

Runtime-specific wrappers are also available:

```bash
bash scripts/install_claude_skills.sh
bash scripts/install_codex_skills.sh
```

## Knowledge Base Layout

```text
kb/
├── ACTIVE.md
├── mission/
│   └── CHALLENGE.md
├── research/
│   ├── hypotheses/
│   ├── experiments/
│   ├── findings/
│   ├── literature/
│   └── data/
├── reports/
└── lessons/
```

### What Each Part Is For

- `kb/ACTIVE.md`: current objective, next step, blocker, working links
- `kb/mission/CHALLENGE.md`: mission brief and success criteria
- `kb/research/*`: the durable research graph
- `kb/reports/`: challenge and strategic reviews
- `kb/lessons/`: small topic files with reusable lessons; not auto-loaded by default

## Shared Runtime Contract

`AGENTS.md` is the shared machine-facing contract.

- Codex reads `AGENTS.md` directly.
- Claude Code reads `CLAUDE.md`, which imports `AGENTS.md`.

This keeps the shared instruction surface short while still letting each runtime add small local notes when needed.

## Runtime Hooks

The core template keeps only deterministic hooks:

- `SessionStart`: inject the mission brief and active state
- `PreToolUse`: block `E` without `H`, and `F` without `E`
- `PostToolUse`: validate kb writes
- `Stop`: run full kb validation before closing

The template intentionally does not include reminder hooks, delegation nudges, or periodic reflection hooks in the core runtime.

## Validator

Run this after substantial kb changes:

```bash
python3 scripts/kb_validate.py
```

The validator checks the research core only:

- required startup files: `kb/ACTIVE.md`, `kb/mission/CHALLENGE.md`
- artifact filenames and directories
- required metadata for `H`, `E`, `F`, `L`, `CR`, `SR`
- research traceability (`E -> H`, `F -> H/E`, `SR -> CR`)
- resolvable wikilinks in `## Links`
- parent backlinks for the core graph

## ID Allocation

There is no manual `Last IDs` ledger.

Use the allocator:

```bash
python3 scripts/kb_next_id.py H
python3 scripts/kb_next_id.py E
python3 scripts/kb_next_id.py F
python3 scripts/kb_next_id.py CR
```

It derives the next ID from the filesystem.

To create a note and wire its first links:

```bash
python3 scripts/kb_new_artifact.py H "Hypothesis title"
python3 scripts/kb_new_artifact.py E "Experiment title" --hypothesis H001
python3 scripts/kb_new_artifact.py F "Finding title" --hypothesis H001 --experiment E001
```

## Obsidian Graph Convention

The kb is meant to be a navigable Obsidian graph, not just a folder of Markdown files.

- Every core artifact aliases its ID in frontmatter.
- Use `[[ID]]` links for artifacts: `[[H001]]`, `[[E003]]`, `[[F010]]`, `[[CR002]]`, `[[SR001]]`.
- Use filename links for fixed notes: `[[ACTIVE]]`, `[[CHALLENGE]]`, `[[DASHBOARD]]`.
- Every core note must contain a `## Links` section.
- Parent and child notes should link to each other.

This keeps the graph easy for agents to maintain and easy for Obsidian to navigate.

## Lessons

Do not keep lessons inside the runtime prompt.

Instead:

- store them in `kb/lessons/`
- keep each file narrow and topic-based
- read only the lesson files relevant to the current problem

This preserves durable learning without forcing a growing lessons ledger into every session.

## Optional Extensions

Optional workflows belong outside the core contract:

- skills
- scoped rules
- project-specific docs
- external orchestration such as `cook`

If a workflow is not essential for every project, it should not live in the main runtime files.

## Included Templates

Core templates live in `templates/`:

- `active.md`
- `hypothesis.md`
- `experiment.md`
- `finding.md`
- `literature.md`
- `challenge-review.md`
- `strategic-review.md`

## Philosophy

Limina is opinionated about a few things:

- durable evidence beats chat memory
- small prompts beat sprawling constitutions
- deterministic hooks beat reminder text
- specific artifacts beat generic wrappers
- research quality matters more than process theater

Everything else is intentionally left lightweight.
