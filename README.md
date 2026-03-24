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

Give Limina a hard technical problem. It will autonomously research it — forming hypotheses, running experiments, challenging its own direction — until it finds a solution that meets your success criteria.

## What is this

Limina is an autonomous AI research agent. You give it a hard technical problem with clear success criteria, and it works through it using a structured multi-agent approach: break the problem down, survey existing work, form hypotheses, design and run experiments, challenge its own direction, and iterate — until it reaches a solution that meets the target, or exhausts the approaches and tells you what it learned.

Everything the agent does is written to a persistent knowledge base (`kb/`). Hypotheses link to experiments. Experiments link to findings. Decisions are logged with reasoning. If the agent gets stuck, it escalates to you instead of guessing. You don't just get a result — you get the full trail of how it got there and why.

## Who is this for

- **Technical leads** — You need to make a decision between approaches and don't have weeks to run the comparison yourself. Limina does the legwork and gives you the evidence to decide.
- **Research engineers** — You're tired of manually setting up experiment after experiment, tracking what you tried, and remembering why you discarded something three days ago. The agent keeps the full trail for you.
- **Scientists** — Your research involves systematic evaluation across many variables. Limina runs the loop — hypothesize, test, record, review, iterate — so you can focus on the questions, not the bookkeeping.
- **Business intelligence** — You have a question that requires more than pulling a dashboard. Something that needs real investigation: gathering data from multiple sources, testing assumptions, building evidence for a recommendation.
- **Anyone with a very hard technical question** — The kind that takes multiple experiments to answer, where you need to track what worked, what didn't, and why. If you've ever lost track of what you already tried, this is for that.

## What you can do with it

**Define a mission.** Describe your research objective — what you're trying to figure out, what "better" means, what resources the agent can use, and when it should come to you for a decision.

**Let it run.** The agent breaks the problem into tasks, forms hypotheses, runs experiments, and iterates toward your success criteria. It works across hours or days and picks up where it left off after interruptions.

**Watch it work.** The Observatory app shows progress as it happens — hypotheses being tested, experiments completing, findings emerging.

**Steer when needed.** When the agent hits something it can't decide on its own — needs more budget, wants to try a risky approach, reached a fork — it stops and asks you. You can also send directives at any time to change the agent's focus, priorities, or approach — and it will incorporate them at the next phase boundary.

**Get the result.** When the agent meets your success criteria — or determines it can't — you have the solution, the full research trail, and the reasoning behind every decision it made along the way.

---

## Quick start

Requires [Node.js 20+](https://nodejs.org/).

```bash
npm install -g @theagilemonkeys/limina
```

Then go to the directory where you want your research project to live:

```bash
cd ~/my-research
limina
```

That's it. `limina` will check for required tools ([Claude Code](https://docs.anthropic.com/en/docs/claude-code), git) and offer to install anything that's missing — including walking you through Claude Code authentication and setting up your Anthropic API key. Then it sets up your mission interactively, launches the agent, and opens the Observatory.


After setup, it stays simple:

```
limina           →  Auto-detects state:
                    No mission?  → Sets one up (interactive)
                    Mission idle? → Starts the agent + Observatory
                    Already running? → Shows status

limina stop      →  Pause anytime, state is preserved
limina doctor    →  Check system health and install missing tools
```

## Writing a good mission

`limina` will ask you to describe your problem. Think of it as a **research brief**, not a coding request. The agent should understand:

1. **Research objective** — what problem it is trying to solve or improve
2. **Evaluation target** — what "better" means and what failure is unacceptable
3. **Baseline** — the current system, method, or repo it should beat or replace
4. **Resource envelope** — what compute, budget, datasets, APIs, and services it can use
5. **Autonomy boundaries** — what it is allowed to generate on its own (evaluation sets, synthetic data, benchmarks)
6. **Escalation rules** — when it should ask the human for more budget, tools, or approvals

### Example

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

This works because it gives the agent a concrete objective, a measurable bar, a baseline to beat, a resource envelope, ownership of evaluation setup, and a clear rule for when to escalate.

## The Observatory

When the agent is running, the Observatory at `localhost:3000` lets you follow along:

- **Dashboard** — mission status, current phase, how many artifacts have been created, elapsed time
- **Directive** — send strategic instructions to the agent mid-loop, respond when it needs your judgment
- **Research** — hypotheses, experiments, and literature as they appear
- **Findings** — results ranked by impact
- **Cost** — budget tracking
- **Report** — final research report with recommendations

## How the agent works

```
limina
    │
    ├── Opens the Observatory
    │
    └── Launches the research agent
         │
         ├── Reads your problem description
         ├── Creates a research plan
         ├── Generates hypotheses (H001, H002, ...)
         ├── Runs experiments (E001, E002, ...)
         ├── Documents findings (F001, F002, ...)
         ├── Asks you when it gets stuck
         └── Writes the final report
```

All artifacts are markdown files in a `kb/` directory. The Observatory watches this directory and shows progress in real-time. Under the hood, the agent uses the [Claude Agent SDK](https://docs.anthropic.com/en/docs/claude-code) (`@anthropic-ai/claude-agent-sdk`) — Claude Code's capabilities as a library — running work-review-iterate cycles with session continuity across iterations.

## Full-manual mode

If you prefer to run the research framework directly — without the Observatory — you can use the `framework/` directory as a standalone template. This is how the [original version](https://github.com/theam/autonomous-researcher/tree/main) of this project works.

### Setup

`limina` handles prerequisites and scaffolding automatically. If you prefer full control, set up manually:

```bash
# Copy the framework into your research directory
cp -r framework/ my-research/
cd my-research

# Write your research objective
# Edit kb/mission/CHALLENGE.md with your problem description

# Install prerequisites
npm install -g @anthropic-ai/claude-code   # Claude Code
```

### Running with Claude Code directly

You can run Claude Code directly without the Observatory:

```bash
claude --dangerously-skip-permissions
```

Then give it your research brief as the first prompt. The `CLAUDE.md` in the framework directory will be loaded automatically and guide the agent through the full methodology.

### What you get

- `CLAUDE.md` — full research methodology (hypothesis-experiment-finding chains, strategic reviews, task system)
- `AGENTS.md` — Codex/OpenCode runtime adapter for the same methodology
- `templates/` — artifact templates for all KB file types
- `scripts/kb_validate.py` — read-only KB validator

## CLI reference

| Command | Description |
|---|---|
| `limina` | Does the right thing — sets up, starts, or shows status |
| `limina init` | Just the setup step (if you want to configure before starting) |
| `limina start` | Just the launch step (agent + Observatory) |
| `limina stop` | Stop the daemon (state is preserved) |
| `limina status` | Show mission phase, artifact counts, daemon status |
| `limina budget [amount]` | View or update the mission budget |
| `limina doctor` | Check prerequisites and install missing tools |

## Configuration

Set in `.env.local` (see `.env.example`):

| Variable | Required | Description |
|---|---|---|
| `MISSION_API_KEY` | Yes | Shared secret for API authentication |
| `ANTHROPIC_API_KEY` | Yes | Anthropic API key for the Claude Agent SDK ([get one here](https://console.anthropic.com/settings/keys)) |
| `SLACK_WEBHOOK_URL` | No | Slack webhook for notifications when the agent needs your input |
| `APP_URL` | No | Base URL for links in notifications (default: `http://localhost:3000`) |

## Contributing

```bash
npx vitest run        # Unit tests
npx playwright test   # E2E tests
npm run build         # Production build
```

Deployment requires a long-running server (`npm start` or Docker) — serverless platforms won't work because the agent needs a persistent process.

## Architecture

Limina uses the [Claude Agent SDK](https://docs.anthropic.com/en/docs/claude-code) (`@anthropic-ai/claude-agent-sdk`) to run the research agent directly as a library — no external orchestration CLI needed. The agent runs in the web server process with session continuity across iterations, enabling mid-loop directive injection.

```
Observatory (Next.js)
  ├── /api/status     → polls kb/ for artifacts
  ├── /api/directive  → sends instructions to the agent mid-loop
  └── MissionRunner   → manages the Claude Agent SDK session
       ├── work phase    → agent follows CLAUDE.md methodology
       ├── directive     → queued directives delivered between phases
       └── review phase  → agent checks if mission is achieved
```
