@AGENTS.md

# Limina

Autonomous research agent web app. Next.js single-process architecture.

## Architecture

Single Next.js app. No database — filesystem-based state (`missions/{id}/kb/` + `mission.json`). Polls KB for changes every 5s. Spawns `cook` CLI via `child_process` for research execution. Claude Code only (v1).

Key modules:
- `src/lib/mission-runner.ts` — MissionRunner class + agent session lifecycle via Claude Agent SDK
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
