# Autonomous Researcher

Autonomous Researcher is an open-source **template framework** for running long-lived research and engineering work with an AI agent backed by a persistent file-based knowledge base.

It is designed to be **compatible with both Codex and Claude Code**:

- `AGENTS.md` is the runtime adapter for **Codex**
- `CLAUDE.md` is the runtime adapter for **Claude Code**

This repository is a **template/starter system**, not a battle-tested project instance. The goal is to give you a reusable operating model, artifact system, and validation layer that you can adapt to a real project.

## What You Get

- A persistent knowledge base in `kb/`
- A dual workflow:
  - research: `H -> E -> F`
  - engineering: `INV -> FT -> IMP -> RET`
- First-class review artifacts: `CR` and `SR`
- Runtime-specific adapters for Codex and Claude Code
- Core artifact templates
- A read-only KB validator: `python3 scripts/kb_validate.py`
- Optional Notion export tooling

## Compatibility

Autonomous Researcher keeps the same functional contract across both supported runtimes while adapting to their native mechanics.

| Capability | Claude Code | Codex |
|---|---|---|
| Ask the user for missing information | `AskUserQuestion` | `request_user_input` or a direct question |
| Delegate work | Slash commands and Claude agents | `spawn_agent` / `send_input` |
| Communicate status | Active session/chat | Active session/chat |
| Validate KB state | `python3 scripts/kb_validate.py` | `python3 scripts/kb_validate.py` |

`README.md` is the canonical human-readable spec. Runtime files are adapters, not the primary place for conceptual explanation.

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

## Quick Start

1. Start with `README.md`, then read the runtime adapter you will use:
   - Codex: `AGENTS.md`
   - Claude Code: `CLAUDE.md`
2. Define your challenge in `kb/mission/CHALLENGE.md` and break it into tasks in `kb/mission/BACKLOG.md`.
3. Use the templates in `templates/` to create the first task and its downstream artifacts.
4. Keep durable progress in `kb/`, not only in chat.
5. Run the validator before closing KB-heavy work:

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

Your first prompt matters. A strong first prompt gives the agent enough context to start working like a director instead of a generic assistant.

Include these elements:

1. **Mission**: what system or outcome you want built or improved.
2. **Quality bar**: what “success” means and what failure is unacceptable.
3. **Baseline**: the current system, repo, or approach the agent should beat or replace.
4. **Operational constraints**: latency, cost, precision/recall, multilingual support, reliability, etc.
5. **Available resources**: datasets, API keys, compute budget, services, and where they live.
6. **Evaluation responsibility**: whether the agent must generate datasets, queries, benchmarks, or test cases.
7. **Escalation rules**: when it should ask for more budget, tooling, or access.

### Good First-Prompt Pattern

Write the prompt as a compact project brief. For example:

```text
Your objective is to improve a multilingual product-discovery system for a retail catalog.

The system should support both natural-language intent queries and traditional keyword search.
Success requires high precision, high recall, and strong latency. Missing relevant items or returning irrelevant ones is not acceptable.

You have an existing baseline service to improve.
You may use the datasets and API keys available in the project environment.
You also have a bounded compute budget and should optimize for effective iteration, not long expensive runs by default.

If evaluation data does not exist, generate it yourself and document how it was created.
If additional tools or budget are needed, ask with a clear justification.
```

That pattern works well because it gives the agent:

- a concrete mission
- explicit non-negotiable constraints
- a starting baseline
- a resource envelope
- ownership of evaluation
- permission to escalate when justified

## License

MIT, © The Agile Monkeys. See [LICENSE](./LICENSE).
