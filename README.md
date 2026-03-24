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

An autonomous AI research agent. Give it a hard technical problem, walk away, come back to a structured research report — hypotheses tested, experiments run, findings ranked by impact.

## How it works

```
limina init      →  You describe the problem
limina start     →  Agent researches autonomously for hours/days
                    Observatory opens at localhost:3000
                    ┌─────────────────────────────────┐
                    │  Dashboard · Research · Findings │
                    │  Steering · Cost · Report        │
                    └─────────────────────────────────┘
limina status    →  Check progress from the terminal
limina stop      →  Pause anytime, state is preserved
```

The agent uses the [cook](https://rjcorwin.github.io/cook/) CLI to orchestrate Claude Code through a structured research loop: hypothesize, experiment, document findings, review, iterate. All state lives on the filesystem in a `kb/` knowledge base — if it's not in `kb/`, it didn't happen.

## Quick start

```bash
# Install
git clone https://github.com/theam/autonomous-researcher.git
cd autonomous-researcher
npm install

# Prerequisites
npm install -g @let-it-cook/cli   # cook CLI for agent orchestration

# Create a research mission
mkdir my-research && cd my-research
limina init

# Launch
limina start
```

## Observatory

The web UI at `localhost:3000` gives you a live view of the agent's work:

- **Dashboard** — mission status, phase, artifact counts, elapsed time
- **Research** — hypotheses, experiments, and literature as they're created
- **Findings** — results ranked by impact
- **Steering** — respond to agent escalations (CEO Requests) when it needs human judgment
- **Cost** — budget tracking
- **Report** — final research report with recommendations

## CLI reference

| Command | Description |
|---|---|
| `limina init` | Interactive setup — describe your problem, set budget, configure Slack |
| `limina start` | Launch the research agent + observatory |
| `limina stop` | Stop the daemon (state is preserved) |
| `limina status` | Show mission phase, artifact counts, daemon status |
| `limina budget [amount]` | View or update the mission budget |

## How the agent works

```
limina start
    │
    ├── Starts Next.js observatory (web UI)
    │
    └── Spawns cook CLI
         │
         └── Claude Code researches autonomously
              │
              ├── Reads CHALLENGE.md (your problem)
              ├── Creates research plan in BACKLOG.md
              ├── Generates hypotheses (H001, H002, ...)
              ├── Runs experiments (E001, E002, ...)
              ├── Documents findings (F001, F002, ...)
              ├── Escalates via CEO_REQUESTS.md when blocked
              └── Writes final report
```

All artifacts are markdown files in `kb/`. The observatory polls this directory and renders progress in real-time.

## Project structure

```
cli/                                CLI commands (init, start, stop, status, budget)
bin/limina                          Entry point
framework/                          Research framework files scaffolded into new missions
src/
├── app/                            Next.js observatory pages
│   ├── page.tsx                    Dashboard
│   ├── research/                   Research artifacts view
│   ├── findings/                   Findings ranked by impact
│   ├── steering/                   CEO request / escalation UI
│   ├── cost/                       Budget tracking
│   ├── report/                     Final report view
│   └── api/                        API routes (status, feedback, escalation, etc.)
├── lib/
│   ├── cook-manager.ts             Cook subprocess lifecycle + state machine
│   ├── kb-parser.ts                Shared parser for all KB artifact types
│   ├── mission.ts                  Mission CRUD, templates, atomic writes
│   └── notify.ts                   Slack webhook notifications
tests/
├── lib/                            Unit tests (Vitest)
└── e2e/                            E2E tests (Playwright)
```

## Configuration

Set in `.env.local` (see `.env.example`):

| Variable | Required | Description |
|---|---|---|
| `MISSION_API_KEY` | Yes | Shared secret for API authentication |
| `SLACK_WEBHOOK_URL` | No | Slack incoming webhook for escalation notifications |
| `APP_URL` | No | Base URL for links in Slack messages (default: `http://localhost:3000`) |

## Testing

```bash
npx vitest run        # Unit tests
npx playwright test   # E2E tests
```

## Deployment

**Must use `next start` or Docker.** Vercel serverless won't work — the agent subprocess and filesystem state require a long-lived Node.js process.

```bash
npm run build && npm start
```
