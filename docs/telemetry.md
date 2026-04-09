# Limina Telemetry

Limina telemetry is explicit opt-in and privacy-constrained by a machine-readable contract at [telemetry/contract.v1.json](../telemetry/contract.v1.json).

## Consent

- Default state is `unset`.
- The first-run prompt offers only three choices: `Share Anonymous Usage`, `Not Now`, and `Never Ask Again`.
- `Share Anonymous Usage` enables telemetry with per-session identity in PostHog.
- `Not Now` keeps telemetry off and suppresses the prompt for 30 days or until the major Limina version changes.
- `Never Ask Again` disables telemetry and clears queued events.
- `Community` remains available only as an explicit advanced opt-in via `python3 scripts/telemetry.py consent --tier community`.

## What Leaves The Machine

Only deterministic usage data leaves the machine:

- runtime family
- artifact type
- validator and guard result codes
- coarse duration buckets
- graph-health counts and booleans

## What Never Leaves The Machine

The client rejects any event payload that tries to include:

- project or repository names
- branches, file paths, URLs, or wikilinks
- code, prompts, or chat transcripts
- knowledge-base note titles, IDs, or bodies
- raw validator output or stack traces
- raw command arguments, tool input, or tool output

## Relay Boundary

Limina clients never talk to PostHog directly. They send sanitized events to the private `limina-telemetry` relay, which enforces the same contract before forwarding accepted events to PostHog.
