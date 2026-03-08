# Autonomous Researcher

Autonomous Researcher is a framework for building **long-running autonomous research agents**.

It is built for agents that can stay focused on a research objective over time, accumulate knowledge, design and run experiments, evaluate progress, challenge their own direction, and keep improving with minimal human involvement.

The human defines the mission, constraints, resources, and intervention thresholds. The agent handles the research loop: breaking work into tasks, forming hypotheses, running experiments, recording findings, and continuing from persistent state across sessions.

Progress means better experiments, better findings, stronger baselines, better decisions, and a durable research history that survives interruptions.

This repository packages that operating model with persistent memory, explicit artifact trails, and validation rules so the research process remains reproducible instead of collapsing into chat history.

It is compatible with **both Codex and Claude Code**:

- `AGENTS.md` is the runtime adapter for **Codex**
- `CLAUDE.md` is the runtime adapter for **Claude Code**

This repository is a **template/starter system**, not a battle-tested deployment. The repository history reflects the evolution of the system over multiple iterations.

## How the Loop Works

1. The human defines the research objective, constraints, and available resources.
2. The agent decomposes the work into tasks, questions, and hypotheses.
3. The agent runs experiments, gathers evidence, and records findings.
4. The agent reviews the direction, challenges assumptions, and updates the plan.
5. The agent continues from persistent state across sessions instead of starting over.

## Why This Exists

Most agent repos stop at one of these layers:

- prompt engineering
- agent orchestration
- experiment scripts

Autonomous Researcher is different because it is a **persistent research operating model**:

- not just a prompt
- not just a loop
- not just a benchmark harness
- a framework for sustained research progress with memory, validation, and review

The point is not only to run tasks autonomously, but to make **measurable research progress** over long periods with a system that can preserve context, justify decisions, and recover after interruptions.

## Compatibility

Autonomous Researcher keeps the same functional contract across both supported runtimes while adapting to their native mechanics.

| Capability | Claude Code | Codex |
|---|---|---|
| Ask the user for missing information | `AskUserQuestion` | `request_user_input` or a direct question |
| Delegate work | Slash commands and Claude agents | `spawn_agent` / `send_input` |
| Communicate status | Active session/chat | Active session/chat |
| Validate KB state | `python3 scripts/kb_validate.py` | `python3 scripts/kb_validate.py` |

`README.md` is the canonical human-readable spec. Runtime files are adapters, not the primary place for conceptual explanation.

## What You Get

- A persistent knowledge base in `kb/`
- A research-first workflow:
  - research: `H -> E -> F`
  - engineering support: `INV -> FT -> IMP -> RET`
- First-class review artifacts: `CR` and `SR`
- Runtime-specific adapters for Codex and Claude Code
- Core artifact templates
- A read-only KB validator: `python3 scripts/kb_validate.py`
- Optional Notion export tooling

## Quick Start

1. Start with `README.md`, then read the runtime adapter you will use:
   - Codex: `AGENTS.md`
   - Claude Code: `CLAUDE.md`
2. Define your challenge in `kb/mission/CHALLENGE.md`.
3. Break the work into tasks in `kb/mission/BACKLOG.md`.
4. Use the templates in `templates/` to create the first task and its downstream artifacts.
5. Keep durable progress in `kb/`, not only in chat.
6. Run the validator before closing KB-heavy work:

```bash
python3 scripts/kb_validate.py
```

The validator is read-only in v1. It checks:

- last-ID declarations in `BACKLOG.md`
- task file and backlog row consistency
- `INDEX.md` coverage for core artifact files
- research traceability: `E -> H`, `F -> E/H/T`
- engineering traceability across `INV`, `FT`, `IMP`, `RET`
- `CR` and `SR` metadata and naming
- malformed filenames, duplicate IDs, and ID gaps

## Writing the First Prompt

Your first prompt should read like a **research brief**, not like a generic coding request.

The agent should understand:

1. **Research objective**: what problem it is trying to solve or improve.
2. **Evaluation target**: what “better” means and what failure is unacceptable.
3. **Baseline**: the current system, method, or repo it should beat or replace.
4. **Resource envelope**: what compute, budget, datasets, APIs, and services it can use.
5. **Autonomy boundaries**: what it is allowed to generate on its own, such as evaluation sets, synthetic queries, or benchmarks.
6. **Escalation rules**: when it should ask the human for more budget, tools, or approvals.

### Good First-Prompt Pattern

Write the first prompt as a compact mission brief. For example:

```text
Your objective is to improve a multilingual retrieval system for a product catalog.

The system should support both natural-language intent queries and traditional keyword search.
Success requires high precision, high recall, and strong latency. Missing relevant items or returning irrelevant ones is not acceptable.

You have an existing baseline system to improve.
You may use the datasets, services, and API keys available in the project environment.
You also have a bounded compute budget and should optimize for effective iteration, not long expensive runs by default.

If evaluation data does not exist, generate it yourself and document how it was created.
If additional tools, budget, or access are needed, ask with a clear justification.
```

That pattern works well because it gives the agent:

- a concrete research objective
- a measurable quality bar
- a baseline to improve
- a resource envelope
- ownership of evaluation setup
- a clear rule for when to escalate to the human

## Core Model

The system is built around a persistent knowledge base in `kb/`.

- Durable state lives in `kb/`, not only in conversation context
- Every unit of work is a task
- Research tasks follow `H -> E -> F`
- Engineering tasks follow `INV -> FT -> IMP -> RET`
- Reviews are first-class artifacts: `CR` and `SR`
- Communication is transport-neutral: use the active session by default
- `DECISIONS.md` and `CEO_REQUESTS.md` are mission ledgers, not file-backed artifact types

### Core Tracked Artifacts

These are the file-backed artifact types enforced by the validator:

| Prefix | Meaning | Location |
|---|---|---|
| `T` | Task | `kb/tasks/` |
| `H` | Hypothesis | `kb/research/hypotheses/` |
| `E` | Experiment | `kb/research/experiments/` |
| `F` | Finding | `kb/research/findings/` |
| `L` | Literature review | `kb/research/literature/` |
| `FT` | Feature spec | `kb/engineering/features/` |
| `INV` | Investigation | `kb/engineering/investigations/` |
| `IMP` | Implementation log | `kb/engineering/implementations/` |
| `RET` | Retrospective | `kb/engineering/retrospectives/` |
| `CR` | Challenge review | `kb/reports/` |
| `SR` | Strategic review | `kb/reports/` |

Generic milestone notes are intentionally excluded from the core validated graph. `templates/report.md` is kept as optional support documentation.

## License

MIT, © The Agile Monkeys. See [LICENSE](./LICENSE).
