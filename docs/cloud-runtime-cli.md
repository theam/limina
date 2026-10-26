# Limina CLI user story

I create a project by telling Limina what outcome matters, what success means, which managed engine
to use, and what context it should respect. I grant named resources. I start the project and leave;
Limina owns the Codex or Claude Code runtime and continues from durable state.

Later I review what it established, steer it asynchronously, or enter the live project and watch it
work. I type normal strategic guidance. I never select a thread, recover a session, dispatch a
subagent, claim a lease, or write a checkpoint.

## Start one server

The localhost-only stack starts without configuration:

```bash
docker compose up --build
```

Then authenticate Codex with a ChatGPT account, or configure engine credentials in `.env`:

```dotenv
OPENAI_API_KEY=...       # optional Codex API-key mode
ANTHROPIC_API_KEY=...    # when using Claude Code
```

```bash
limina runtime codex login                 # ChatGPT device flow
limina runtime codex login --method api-key
```

When local access tokens are desired, keep project and runtime-administration authority separate:

```dotenv
LIMINA_API_TOKEN=replace-with-a-long-random-project-value
LIMINA_ADMIN_API_TOKEN=replace-with-a-different-admin-value
```

The `limina-data` volume preserves SQLite, project workspaces, both providers' private continuation
stores, provider continuation pointers, and the encryption key. The image applies schema
migrations before serving; there is no separate database or runtime command in this mode.

The direct development path is:

```bash
export LIMINA_API_TOKEN=local-secret
export LIMINA_ADMIN_API_TOKEN=another-local-secret
cargo run -p limina-cli -- serve
```

For PostgreSQL:

```bash
docker compose -f compose.cloud.yaml up --build
```

Project commands use `LIMINA_URL` (default `http://127.0.0.1:7433`), `LIMINA_API_TOKEN`, and the
optional attribution identity `LIMINA_ACTOR`. Runtime administration uses
`LIMINA_ADMIN_API_TOKEN`.

## Choose a runtime

```bash
limina project create retrieval \
  --runtime claude-code \
  --name "Retrieval quality" \
  --mission "Improve multilingual product retrieval" \
  --success "Increase held-out NDCG by 10% without a P95 latency regression" \
  --context "The current baseline is hybrid BM25 plus embeddings"
```

Valid values are `codex` and `claude-code`; `codex` is the default. `limina doctor` reports what
the server supports, and `project show`/`project list` show each project's selection.

A project's runtime cannot be changed. Create a separate project for an engine comparison or a
fresh continuation. Users choose the product-level engine, not its model session.

## Provide variables and secrets

```bash
limina resource variable retrieval SOURCE_REPO_URL https://github.com/acme/search
limina resource variable retrieval EVAL_SET_URI s3://research/eval-v3.parquet

limina resource secret retrieval GITHUB_TOKEN --from-env GITHUB_TOKEN
limina resource secret retrieval AWS_SESSION_TOKEN --from-env AWS_SESSION_TOKEN

limina resource list retrieval
```

Variables are visible configuration. Secrets are encrypted and write-only: omit `--from-env` for
a hidden confirmation prompt, or use `--from-stdin` in automation. Limina injects both only into
the selected runtime for that project. A change during active work is committed, then Limina
restarts the managed turn with a freshly materialized environment. Use TLS whenever the API is
reachable beyond localhost.

## Start and leave it running

```bash
limina start retrieval
limina status retrieval
limina watch retrieval
```

`watch` follows durable activity. `Ctrl-C` stops watching; it does not stop the project. Limina
recovers active projects after a server restart and resumes their private provider continuation.

## Steer asynchronously

```bash
limina steer retrieval "The latency guardrail is more important than the 10% target."
limina steer retrieval "Approved to spend up to $100 on the larger evaluation." --kind APPROVAL
```

Limina commits guidance before attempting live delivery. Codex accepts active-turn steering.
Claude Code is interrupted and immediately redirected inside the same session. Otherwise the
message remains pending and wakes the next managed turn.

## Enter the live project

```bash
limina attach retrieval
```

The terminal shows current state, follows durable activity, and accepts direct feedback:

```text
limina> Compare against the cross-encoder baseline before drawing a conclusion.
limina> /pause
limina> /resume
limina> /interrupt
limina> /detach
```

- plain text steers active work or queues durably if the turn ended;
- `/pause`, `/resume`, and `/stop` change project lifecycle;
- `/interrupt` interrupts active work and pauses the project;
- `/detach` leaves the view while execution continues.

Several teammates may attach simultaneously. Each sees the same persisted event sequence, and
each message carries their `--actor`/`LIMINA_ACTOR` identity.

## Review accepted knowledge

```bash
limina review retrieval
limina review retrieval --artifact F003
limina export retrieval ./retrieval-kb
```

Review groups hypotheses, experiments, and findings. Opening an artifact shows accepted content
and evidence. Export creates a validator-compatible Markdown KB for offline review, Obsidian,
archival, or a Git review branch.

## Manage projects

```bash
limina project list
limina project show retrieval
limina pause retrieval
limina resume retrieval
limina stop retrieval
limina project archive retrieval
```

Stopping and archiving never delete knowledge. Archive requires the project to be inactive.

## Machine-readable use

Non-interactive public commands support global `--json`:

```bash
limina --json project show retrieval | jq '{runtime, status, next_step}'
limina --json review retrieval | jq '.findings[] | {id, title}'
```

`attach` is interactive and rejects `--json`; event consumers should use
`watch --no-follow --json`.

Services can use the same versioned REST contract directly, and collaborating agents can connect
to the authenticated Streamable HTTP endpoint at `/mcp/`. Both interfaces call the same public
project operations as the CLI. MCP exposes project management, steering, review, activity,
variables, and read-only knowledge resources; secret values remain restricted to the trusted CLI
or REST path so they do not enter an agent's model-visible tool arguments. See the root README for
ready-to-use Codex and Claude Code connection examples.

## What users never operate

Limina contains a hidden, project-scoped command protocol so either managed runtime can commit
H → E → F artifacts safely. It is omitted from public help, OpenAPI, and MCP, protected by a
short-lived capability, and not part of the user workflow. If a human is managing sessions,
threads, workers, subagents, leases, versions, or checkpoints, the abstraction has failed.
