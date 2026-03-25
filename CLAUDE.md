@AGENTS.md

# Limina

Autonomous research agent web app. Two-service architecture.

## Architecture

Two services: Next.js UI server + Fastify agent service. No database — filesystem-based state (`missions/{id}/kb/` + `mission.json`). UI polls KB for changes every 5s. Agent service runs the Claude Agent SDK in a separate process. Services communicate via HTTP.

```
CLI (supervisor)
  ├── Next.js server (UI — reads filesystem)
  └── Agent service  (Fastify — runs MissionRunner, writes filesystem)
```

Key modules:
- `src/agent-service/server.ts` — Fastify HTTP server wrapping MissionRunner (start/kill/directive/health endpoints)
- `src/lib/mission-runner-core.ts` — MissionRunner class + agent session lifecycle via Claude Agent SDK
- `src/lib/mission-runner.ts` — HTTP client to agent service (same API surface, used by Next.js routes)
- `src/lib/kb-parser.ts` — shared parser for all KB artifact types (H, E, F, CR, SR, T, L)
- `src/lib/mission.ts` — CHALLENGE.md templates, mission CRUD, atomic writes
- `src/lib/notify.ts` — Slack webhook notifications

## Testing

```bash
# Unit tests
npx vitest run

# E2E tests
npx playwright test
```

- Framework: Vitest (unit) + Playwright (E2E)
- Test directory: `tests/`
- 50 unit tests, 19 E2E tests
- When writing new functions, write a corresponding test
- When fixing a bug, write a regression test
- When adding a conditional, write tests for BOTH paths
- Never commit code that makes existing tests fail

## Development

```bash
npm run dev     # Start dev server on :3000
npm run build   # Production build
npm start       # Production server (required — no Vercel serverless)
```

Requires `.env.local` with `MISSION_API_KEY`. See `.env.example`.

## Conventions

- API key auth via `Authorization: Bearer <key>` header
- Mission IDs: `m_` prefix + nanoid (e.g., `m_abc123def4`)
- State transitions validated by `isValidTransition()` — invalid transitions throw
- CEO_REQUESTS.md uses atomic write (temp file + rename) for concurrent access
- Mission state persisted in `missions/{id}/mission.json` (also atomic write)
- KB artifact metadata uses `> **Key**: Value` blockquote format from the framework's templates
