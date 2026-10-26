# Limina runtime (Rust)

The managed runtime and the `limina` command, as one Cargo workspace. It replaced the original
Python runtime and keeps its HTTP contract, database schema and persisted data, so the web console,
existing databases and stored secrets keep working.

## Crates

```text
limina-core     errors, time, Principal                      (no I/O)
  ↑
limina-store    SQLite/PostgreSQL pool, schema, row models, secret cipher
  ↑
limina-domain   projects, H → E → F graph, steering, collaboration, knowledge, export
  ↑
limina-engines  Codex (app-server JSON-RPC) and Claude (CLI stream-json) sessions, event normalization
  ↑
limina-server   axum REST API, live WebSocket, MCP, sign-in (tokens, OIDC, WorkOS), supervisor, console
  ↑
limina-cli      the `limina` binary
```

Each crate depends only on the crates below it.

## Compatibility

The public contract is what clients, the console and stored data depend on. Change it only on
purpose, and say so in the change.

- **Behavior.** Validation rules, state transitions, error codes, HTTP statuses, CLI exit codes and
  error messages are part of the contract; clients and tests match on them.
- **Persisted shapes.** Event payloads, artifact payloads and command receipts are stored as JSON
  and read back later, including rows written by the original Python runtime. A stored type must
  keep serializing with the same keys and value types.
- **Wire format.** JSON field names and nesting are stable. Timestamps are ISO-8601 with `+00:00`
  (`limina_core::iso`, or `#[serde(with = "limina_core::time::serde_iso")]`).
- **Golden fixtures.** The JSON fixtures under `crates/*/tests/fixtures/` were recorded from the
  Python runtime before it was removed (API responses, OpenAPI, MCP tools, prompts, the scripted
  demo). They are frozen: a change that alters them is a contract change.

## Conventions

- **Errors.** Return `limina_core::Result<T>`. Build errors with `LiminaError::not_found(..)`,
  `::conflict`, `::invariant`, … and attach details with `.with(key, value)`. Database errors convert
  with `?`; a unique or foreign-key violation becomes a conflict with `Cause::Constraint`.
- **Commands.** Every write the API exposes is idempotent on a client-supplied command ID. Wrap it
  in `limina_domain::command!(db, command_id, "kind.verb", actor, |tx| { … })`; the block runs in one
  write transaction and its result is stored as the receipt.
- **SQL.** One dialect for both backends, `$N` placeholders, values always bound (`args![..]`),
  never interpolated. Follow the store crate docs: `INTEGER` → `i32`, `BIGINT` → `i64`, cast integer
  aggregates to `BIGINT`, append `tx.for_update()` for row locks, use `db.begin_write()` for writes.
- **Time.** `limina_core::now()` (microsecond precision). Never format timestamps by hand.
- **No panics in library code.** `unwrap`/`expect`/`panic!` are linted outside tests. No `unsafe`.
- **Async.** Tokio everywhere. Blocking work (files, process waits) goes through `spawn_blocking`
  or Tokio's async equivalents; nothing blocks the runtime.
- **Dependencies.** Prefer well-maintained crates already in the workspace. Add a new one to the
  crate's own `Cargo.toml` with an exact minor version, and say why in the change.
- **Open source.** No private names, internal URLs or company-specific configuration in code,
  tests or docs.

## Checks

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

PostgreSQL tests run when `LIMINA_TEST_POSTGRES_URL` points at an empty database (see
`limina-store/tests/backends.rs`).
