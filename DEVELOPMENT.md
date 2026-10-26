# Development

Common commands for working on Limina locally.

## Commands

```bash
make test
make validate
make generate-goal
make install-skills
```

`make generate-goal` expects `kb/mission/CHALLENGE.md` to contain a real Objective and Success Criteria. The template placeholder intentionally fails with a friendly message.

## Web console

```bash
make web-install   # once
make ui-dev        # runtime on :7433 with a scripted engine (no provider credentials)
make web-dev       # Vite on :5173 with hot reload → http://127.0.0.1:5173/#token=dev-admin
make web-check     # typecheck, unit tests, production build
```

See [docs/web-console.md](docs/web-console.md) for the architecture and design rules.

## Scripts

- `scripts/generate_goal.py`: builds a Codex `/goal` command from `kb/mission/CHALLENGE.md`.
- `scripts/kb_validate.py`: validates the core research graph and optional `GOAL.md` quality.
- `scripts/install_skills.sh`: symlinks bundled skills into Claude Code and/or Codex.
- `scripts/kb_new_artifact.py`: creates new `H`, `E`, `F`, `L`, `CR`, and `SR` notes.
- `limina dev` (`make ui-dev`): runs the managed runtime with a scripted engine for UI work and
  demos. The runtime itself is the Rust workspace in `crates/` (see `crates/README.md`).

## Testing

Install dependencies, then run:

```bash
python3 -m unittest discover -s tests
python3 scripts/kb_validate.py
```

The Makefile sets `LIMINA_TELEMETRY_INTERNAL=1` for tests and validation so local development checks do not prompt for telemetry consent.

## Release Checks

Before publishing a change:

```bash
make test
make validate
```
