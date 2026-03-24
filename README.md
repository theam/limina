# Limina

Submit a complex technical problem, get an autonomous AI research report back — with hypotheses tested, experiments run, and actionable recommendations.

## Who is this for?

Technical leads and engineering managers who need AI research outcomes but lack access to deep AI research talent. Limina runs autonomously for hours/days, and the web interface makes it accessible without touching the CLI.

## How it works

```
Intake Form → Research Plan → Approve → Live Progress → Report
     │              │              │           │            │
     ▼              ▼              ▼           ▼            ▼
 CHALLENGE.md    BACKLOG.md     cook CLI    KB artifacts   Findings
 generated       auto-plan      spawns      appear via     ranked by
 from form       from agent     research    polling        impact
```

1. **Submit a mission** — describe your problem, success metric, and context
2. **Review the plan** — the AI generates a research plan with tasks, approve or reject
3. **Watch progress** — hypotheses, experiments, and findings appear as the agent works
4. **Respond to escalations** — the agent asks for input when it needs human judgment
5. **Get the report** — key findings ranked by impact, with a recommendation

## Quick start

```bash
# Clone and install
git clone https://github.com/theam/autonomous-researcher.git
cd autonomous-researcher
npm install

# Configure
cp .env.example .env.local
# Edit .env.local — set MISSION_API_KEY to any secret string

# Prerequisites
npm install -g @let-it-cook/cli   # cook CLI for agent orchestration

# Run
npm run dev
# Open http://localhost:3000
```

Set your API key in the browser: open DevTools console and run:
```js
localStorage.setItem('ar-api-key', 'your-secret-key-here')
```

## Architecture

Single Next.js process. No database. Filesystem-based state.

```
┌──────────────────────────────────────────────┐
│            Next.js App (single process)        │
│                                                │
│  Pages:  / (list) → /new (intake) → /missions/[id] (workspace)
│  API:    /api/missions (CRUD, status, escalation, kill, share)
│  Lib:    cook-manager, kb-parser, mission, notify
│                                                │
└────────────────┬───────────────────────────────┘
                 │ spawns via child_process
┌────────────────┴───────────────────────────────┐
│  cook CLI → Claude Code → writes to missions/{id}/kb/
└────────────────────────────────────────────────┘
```

### Key decisions

| Decision | Choice | Why |
|---|---|---|
| Backend | Next.js only | cook is Node.js; one server, one language |
| Auth | API key in `.env` | v1 is single-tenant with 3 design partners |
| Storage | Filesystem | `kb/` IS the persistent state; `mission.json` for metadata |
| Real-time | Polling every 5s | Simpler than SSE, same UX for multi-hour workflows |
| Notifications | Slack webhooks | Nobody watches a page for days |
| Runtime | Claude Code only | One reliable runtime for v1 |

## Configuration

| Variable | Required | Description |
|---|---|---|
| `MISSION_API_KEY` | Yes | Shared secret for API authentication |
| `SLACK_WEBHOOK_URL` | No | Slack incoming webhook for notifications |
| `APP_URL` | No | Base URL for links in Slack messages (default: `http://localhost:3000`) |

## Mission state machine

```
CREATED → PLANNING → PLAN_READY → RUNNING → COMPLETED
                                    ├→ CHECKPOINT_WAITING → RUNNING
                                    ├→ ESCALATION_WAITING → RUNNING
                                    ├→ STALLED
                                    ├→ FAILED_RECOVERABLE → RUNNING
                                    └→ KILLED
```

## API routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/missions` | List all missions |
| `POST` | `/api/missions` | Create a new mission |
| `GET` | `/api/missions/:id/status` | Poll mission state + KB artifacts |
| `POST` | `/api/missions/:id/approve` | Approve plan / resume from checkpoint |
| `POST` | `/api/missions/:id/escalation` | Respond to CEO request |
| `POST` | `/api/missions/:id/kill` | Stop mission (preserves state) |
| `POST` | `/api/missions/:id/share` | Generate shareable report link |
| `GET` | `/api/reports/:token` | Public read-only report (no auth) |

All endpoints except `/api/reports/:token` require `Authorization: Bearer <API_KEY>`.

## Testing

```bash
# Unit tests (Vitest)
npx vitest run

# E2E tests (Playwright)
npx playwright test

# Build check
npx next build
```

50 unit tests + 19 E2E tests. Every module ships with tests.

## Deployment

**Must use `next start` or Docker.** Vercel serverless is NOT compatible (subprocess state and persistent watchers require a long-lived Node.js process).

```bash
npm run build
npm start
```

## Project structure

```
src/
├── app/
│   ├── page.tsx                    Missions list (homepage)
│   ├── new/page.tsx                Intake form
│   ├── missions/[id]/page.tsx      Mission workspace (plan/progress/report tabs)
│   └── api/missions/               API routes
├── lib/
│   ├── cook-manager.ts             State machine + subprocess lifecycle
│   ├── kb-parser.ts                Shared artifact parser (all types)
│   ├── mission.ts                  Templates, state CRUD, atomic writes
│   └── notify.ts                   Slack webhook notifications
missions/                           Runtime data (gitignored)
tests/
├── lib/                            Unit tests (Vitest)
└── e2e/                            E2E tests (Playwright)
```

## Related

- [cook](https://rjcorwin.github.io/cook/) — agent orchestration CLI used for the research loop
