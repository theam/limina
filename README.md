```
  ██╗     ██╗███╗   ███╗██╗███╗   ██╗ █████╗
  ██║     ██║████╗ ████║██║████╗  ██║██╔══██╗
  ██║     ██║██╔████╔██║██║██╔██╗ ██║███████║
  ██║     ██║██║╚██╔╝██║██║██║╚██╗██║██╔══██║
  ███████╗██║██║ ╚═╝ ██║██║██║ ╚████║██║  ██║
  ╚══════╝╚═╝╚═╝     ╚═╝╚═╝╚═╝  ╚═══╝╚═╝  ╚═╝

  from Latin līmen — "threshold"
  Cross the boundary between known and unknown.
```

[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![GitHub stars](https://img.shields.io/github/stars/theam/limina)](https://github.com/theam/limina/stargazers)
[![GitHub forks](https://img.shields.io/github/forks/theam/limina)](https://github.com/theam/limina/network/members)

Built by [The Agile Monkeys](https://theagilemonkeys.com).

Give Limina a problem with a measurable goal. It will autonomously research it — forming hypotheses, running experiments, challenging its own direction — until it finds a solution backed by evidence, or tells you what it learned trying.

## What is this

Limina is an autonomous research harness for AI agents. You describe a problem with clear success criteria, and the agent works through it: break it down, survey existing approaches, form hypotheses, design and run experiments, challenge its own assumptions, and iterate — until it reaches a solution or exhausts the approaches and tells you what it learned.

It works on anything with a measurable outcome. Our team uses it for:

- Optimizing a search engine for a large e-commerce platform
- A/B testing product features
- Researching state-of-the-art approaches for audio transcription
- Optimizing social media reach with data-driven experiments
- Investigating root causes of production performance issues

Everything the agent does is written to a persistent knowledge base (`kb/`). Hypotheses link to experiments. Experiments link to findings. Decisions are logged with reasoning. If the agent gets stuck, it escalates to you instead of guessing. You don't just get a result — you get the full trail of how it got there and why.

This repository is a **template/starter system** — clone it, start an agent, and describe your problem.

## Who is this for

- **Technical leads** — You need to make a decision between approaches and don't have weeks to run the comparison yourself. Limina does the legwork and gives you the evidence to decide.
- **Product teams** — You want to optimize a metric — conversion rate, latency, cost, user engagement — and need systematic experimentation, not guesswork.
- **Research engineers** — You're tired of manually setting up experiment after experiment, tracking what you tried, and remembering why you discarded something three days ago. The agent keeps the full trail for you.
- **Scientists** — Your research involves systematic evaluation across many variables. Limina runs the loop — hypothesize, test, record, review, iterate — so you can focus on the questions, not the bookkeeping.
- **Business intelligence** — You have a question that requires more than pulling a dashboard. Something that needs real investigation: gathering data from multiple sources, testing assumptions, building evidence for a recommendation.
- **Anyone with a goal that can be measured** — If you can define what "better" looks like, Limina can research how to get there.

## What you can do with it

**Define a mission.** Describe your research objective — what you're trying to figure out, what "better" means, what resources the agent can use, and when it should come to you for a decision.

**Let it run.** The agent breaks the problem into tasks, forms hypotheses, runs experiments, and iterates toward your success criteria. It works across hours or days and picks up where it left off after interruptions.

**Steer when needed.** When the agent hits something it can't decide on its own — needs more budget, wants to try a risky approach, reached a fork — it stops and asks you.

**Get the result.** When the agent meets your success criteria — or determines it can't — you have the solution, the full research trail, and the reasoning behind every decision it made along the way.

## Quick start

Open [Claude Code](https://docs.anthropic.com/en/docs/claude-code) or [Codex](https://openai.com/index/introducing-codex/) and paste:

```text
Install the Limina research skill by running:
curl -fsSL https://raw.githubusercontent.com/theam/limina/main/setup.sh | bash
Then ask me to change my Claude Code working directory to the folder where I want
my research project to live, and help me set up a new Limina research project.
```

The agent will install the skill, ask you to switch to your preferred directory, then guide you through everything — project name, research objective, context, success criteria.

When setup is done, open Claude Code in the new project directory:

```bash
cd <your-project-name> && claude
```

The agent reads the methodology automatically and starts researching.

### What to expect

As the agent works, it builds a knowledge base in `kb/`:

```
kb/
├── mission/
│   ├── CHALLENGE.md        ← your research brief
│   └── BACKLOG.md          ← task tracking
├── research/
│   ├── hypotheses/H001.md  ← what it thinks might work
│   ├── experiments/E001.md ← how it tested each hypothesis
│   └── findings/F001.md   ← what it learned
├── reports/
│   └── SR001.md            ← strategic review
└── tasks/
    ├── T001.md
    └── T002.md
```

Check progress anytime by reading the files in `kb/` or asking the agent for a status update. When it gets stuck or needs a decision, it will ask you.

## Writing a good mission

The agent will ask you about your problem interactively. You'll get better results if your description reads like a **research brief** — here's what to include:

1. **Research objective** — what problem you're trying to solve or improve
2. **Evaluation target** — what "better" means and what failure is unacceptable
3. **Baseline** — the current system, method, or repo to beat or replace
4. **Resource envelope** — what compute, budget, datasets, APIs, and services are available
5. **Autonomy boundaries** — what the agent is allowed to generate on its own (evaluation sets, synthetic data, benchmarks)
6. **Escalation rules** — when it should ask you for more budget, tools, or approvals

### Examples

**Research & optimization:**

```text
Your objective is to improve a multilingual retrieval system for a product catalog.

The system should support both natural-language intent queries and traditional keyword search.
Success requires high precision, high recall, and strong latency. Missing relevant items or
returning irrelevant ones is not acceptable.

You have an existing baseline system to improve.
You may use the datasets, services, and API keys available in the project environment.
You also have a bounded compute budget and should optimize for effective iteration, not long
expensive runs by default.

If evaluation data does not exist, generate it yourself and document how it was created.
If additional tools, budget, or access are needed, ask with a clear justification.
```

**Investigation & root cause analysis:**

```text
Our API's P99 latency jumped from 120ms to 800ms after the last deploy.
We need to find the root cause and a fix.

The service is a Node.js app on ECS with a PostgreSQL database.
You have access to the repo, CloudWatch logs, and APM traces.
Success means P99 back under 200ms with the fix verified in staging.

If you need access to production or want to run load tests, ask first.
```

**Product optimization:**

```text
We need to improve the conversion rate of our landing page.
Current conversion is 2.3% and we want to reach 4%.

Run A/B tests on copy, layout, and CTA variations. You can generate
test variants and analyze results from our analytics API.
Track what you tested, what worked, and why.

If you need to deploy a variant to production, ask first.
```

## How it works

```
You describe the problem
  → Agent decomposes into tasks
  → Hypothesis → Experiment → Finding
  → Reviews direction, challenges assumptions
  → Iterates from persistent state across sessions
```

1. You describe the research objective, constraints, and available resources.
2. The agent decomposes the work into tasks, questions, and hypotheses.
3. The agent runs experiments, gathers evidence, and records findings.
4. The agent reviews the direction, challenges assumptions, and updates the plan.
5. The agent continues from persistent state across sessions instead of starting over.

---

## Compatibility

Limina works with Claude Code, Codex, and OpenCode. Claude Code loads `CLAUDE.md` automatically; Codex and OpenCode load `AGENTS.md`. Both files are functionally equivalent — they guide the agent through the same methodology using runtime-specific tools.

| Capability | Claude Code | Codex | OpenCode |
|---|---|---|---|
| Ask the user for missing information | `AskUserQuestion` | `request_user_input` or a direct question | Direct question |
| Delegate work | Slash commands and Claude agents | `spawn_agent` / `send_input` | — |
| Communicate status | Active session/chat | Active session/chat | Active session/chat |
| Validate KB state | `python3 scripts/kb_validate.py` | `python3 scripts/kb_validate.py` | `python3 scripts/kb_validate.py` |

## Autonomous execution with cook

[cook](https://rjcorwin.github.io/cook/) is a universal orchestration CLI that handles work-review-gate cycles across any agent runtime. Use it when you want the agent to run fully autonomously with built-in review gates.

```bash
npm install -g @let-it-cook/cli
```

**Continue research (open-ended):**
```bash
cook "Continue research" review \
     "Review current status and verify if we achieved the target mission" \
     "DONE if we achieved the target mission, else ITERATE"
```

**Research with iteration cap:**
```bash
cook "Continue research" review \
     "Review current status and verify if we achieved the target mission" \
     "DONE if we achieved the target mission, else ITERATE" \
     --max-iterations 10
```

**Mixed agents (Codex work, Claude review):**
```bash
cook "Continue research" review \
     "Review current status and verify if we achieved the target mission" \
     "DONE if we achieved the target mission, else ITERATE" \
     --work-agent codex --review-agent claude
```

**Challenge review:**
```bash
cook "Run /challenge with target 'Research direction'" review \
     "Read the CR report and assess whether critical issues were addressed" \
     "DONE if no critical issues remain, else ITERATE"
```

## What you get

- A persistent knowledge base in `kb/` with Obsidian-compatible YAML frontmatter
- A research-first workflow:
  - research: Hypothesis → Experiment → Finding
  - engineering: Investigation → Feature → Implementation → Retrospective
- **Runtime enforcement hooks** — the H→E→F chain, delegation, and decision review are enforced mechanically, not just by instructions
- First-class review artifacts: Challenge Reviews and Strategic Reviews
- A devil's advocate that reviews every experiment, not just every third
- Adapters for Claude Code, Codex, and OpenCode
- Core artifact templates in `templates/`
- A read-only KB validator: `python3 scripts/kb_validate.py`
- Provenance and staleness tracking: `python3 scripts/kb_provenance.py`
- Optional Obsidian vault integration: `bash scripts/obsidian_init.sh`

## Core model

The system is built around a persistent knowledge base in `kb/`.

- Durable state lives in `kb/`, not only in conversation context
- Every unit of work is a task
- Research tasks follow Hypothesis → Experiment → Finding
- Engineering tasks follow Investigation → Feature → Implementation → Retrospective
- Reviews are first-class artifacts: Challenge Reviews and Strategic Reviews
- `DECISIONS.md` and `CEO_REQUESTS.md` are mission ledgers, not file-backed artifact types

### Core tracked artifacts

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

The validator is read-only. It checks:

- last-ID declarations in `BACKLOG.md`
- task file and backlog row consistency
- `INDEX.md` coverage for core artifact files
- research traceability: experiments link to hypotheses, findings link to experiments
- engineering traceability across investigations, features, implementations, retrospectives
- challenge review and strategic review metadata and naming
- malformed filenames, duplicate IDs, and ID gaps

The validator supports both YAML frontmatter and blockquote metadata formats. Additional modes:

- `--check-file <path>` — validate a single file in isolation (fast, used by hooks)
- `--quiet` — suppress output when validation passes
- `--format json` — output results as JSON

### Runtime enforcement

In Claude Code, hooks in `.claude/settings.json` enforce rules automatically at runtime:

| Hook | Type | What it does |
|---|---|---|
| `session_start.sh` | SessionStart | Injects CLAUDE.md, INDEX.md, and BACKLOG.md into agent context |
| `enforce_hef_chain.sh` | PreToolUse | **Blocks** experiment creation without a hypothesis, and finding creation without an experiment |
| `kb_write_guard.sh` | PostToolUse | Validates every `kb/` write against the artifact schema in real-time |
| `experiment_gate.sh` | PostToolUse | Prompts the agent to evaluate results and spawn the devil's advocate after experiment completion |
| `protocol_checkpoint.sh` | PostToolUse | Reminds the agent to re-read state every 25 tool calls; injects a reflection prompt every 50 |
| `delegation_guard.sh` | PostToolUse | Nudges the Director when it writes execution artifacts directly instead of delegating |
| `decision_critic.sh` | PostToolUse | Prompts a devil's advocate review whenever a decision or finding is recorded |

The hooks are deterministic shell scripts — the agent cannot choose to skip them. Blocking hooks (exit code 2) prevent the action entirely; non-blocking hooks (exit code 0) inject guidance into the agent's context.

### Provenance and staleness

`python3 scripts/kb_provenance.py --stale-check` detects:

- Findings referencing superseded hypotheses
- Decisions citing rejected hypotheses
- Contradictions between findings on the same hypothesis
- Literature entries older than a configurable threshold

### Obsidian integration

`bash scripts/obsidian_init.sh` sets up an optional Obsidian vault over `kb/`:

- Creates `.obsidian/` config with Dataview plugin settings
- Generates a `DASHBOARD.md` with live queries for tasks, experiments, findings, and literature
- Configures graph view color groups (research, engineering, reports)

The `kb/` remains a Git-backed Markdown source of truth. Obsidian is the human UI layer.

## Contributing

Found a bug? Have an idea? We'd love your input.

- [Open an issue](https://github.com/theam/limina/issues) to report problems or suggest features
- [Start a discussion](https://github.com/theam/limina/discussions) to ask questions or share how you're using Limina

## License

Apache 2.0, © The Agile Monkeys. See [LICENSE](./LICENSE).
