//! The runtime's application layer: the project supervisor, per-project engine accounts and the
//! operations facade the HTTP API and the MCP server call.
//!
//! A port of `supervisor.py`, `engine_accounts.py`, `operations.py` and the non-HTTP half of
//! `create_app` (`api.py`) from the Python runtime, with the same rules, messages and JSON.
//!
//! ```text
//! Runtime                       built from a RuntimeConfig (environment or explicit fields)
//!  ├─ ChallengeService, CollaborationService, MarkdownExporter        (limina-domain)
//!  ├─ ProjectSupervisor        one Tokio task per active project; provider sessions, turns,
//!  │   │                       leases, retries, capability tokens for the internal agent API
//!  │   ├─ ProjectEngineAccounts   project Claude tokens (encrypted) and Codex homes
//!  │   └─ CodexAuthRouter         the runtime's Codex sign-in, or the project's own
//!  ├─ ProjectOperations        role checks, per-identity idempotency, public views, side
//!  │                           effects on the supervisor (start, interrupt, resource refresh),
//!  │                           sharing by email ([`sharing`])
//!  └─ EventNotifier            per-project "new events" signal for live clients
//! ```
//!
//! # What the transports call
//!
//! - [`Runtime::new`] with [`RuntimeConfig::from_env`] (or explicit fields), [`Runtime::start`]
//!   once serving begins and [`Runtime::shutdown`] when it ends.
//! - [`ProjectOperations`] for every public route and MCP tool: each method takes the caller's
//!   [`limina_core::Principal`] and returns a `Serialize` type whose JSON is the Python API's
//!   response body ([`views`]). Errors are [`limina_core::LiminaError`]s with the Python codes
//!   and messages; the HTTP status is [`limina_core::LiminaError::http_status`].
//! - [`Runtime::internal_actor`] for the internal agent API (`/internal/v1/…`): it turns the
//!   `Authorization: Bearer <capability>` and `X-Limina-Agent-Lane` headers into the actor the
//!   internal command runs as, or an `authentication_required` error.
//! - [`EventNotifier::subscribe`] for the live WebSocket (see [`notifier`] for the loop).
//!
//! # Environment
//!
//! | Variable | Default | Meaning |
//! |---|---|---|
//! | `LIMINA_DATABASE_URL` | `sqlite:///.limina/runtime.db` | SQLite or PostgreSQL |
//! | `LIMINA_WORKSPACE_ROOT` | `.limina/workspaces` | project workspaces; its parent is the state root |
//! | `LIMINA_SECRET_KEY` | — | Fernet key for secrets and project tokens |
//! | `LIMINA_SECRET_KEY_PATH` | `<state root>/secret.key` | key file used without `LIMINA_SECRET_KEY` |
//! | `LIMINA_INTERNAL_URL` | `http://127.0.0.1:7433` | the internal agent API, for providers |
//! | `LIMINA_RUNTIME_RETRY_DELAYS_SECONDS` | `30,120,600` | durable retry back-off |
//! | `LIMINA_UPLOAD_LIMIT_BYTES` | `26214400` | largest uploaded source |
//! | `LIMINA_CODEX_*`, `CODEX_HOME`, `LIMINA_CLAUDE_*`, `OPENAI_API_KEY`, `CODEX_ACCESS_TOKEN` | | engines ([`limina_engines::EngineSettings`]) |
//! | `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN`, `CLAUDE_CODE_USE_*` | | server Claude credentials, read when needed |
//!
//! Provider processes additionally get `PATH` with the running executable's directory first (see
//! [`supervisor`]).
//!
//! # Deliberate differences from the Python runtime
//!
//! - **`PATH` for providers** starts with the running executable's directory, so the agent's
//!   `limina _agent …` commands always run the server's own build.
//! - **Slugs are canonical** (lower-case) for loop keys, workspaces, Codex homes and the binding
//!   of a project's Claude token. Python used the slug as the caller spelled it, so a mixed-case
//!   URL could start a second loop or bind a token to the wrong spelling.
//! - **An `INTERRUPT` pauses before it interrupts.** Python's synchronous database calls made the
//!   pause land before the interrupted turn looked at the project; with async writes the order is
//!   now explicit, so the turn reliably records `INTERRUPTED` rather than `FAILED`.
//! - **One session per project.** Python called the agent factory each time a loop started and
//!   discarded the extra session.
//! - **Containment.** Errors Python let escape a project's loop (status reads, event writes inside
//!   failure handling) are logged and end only that loop; failure recording continues past a
//!   write that fails. A panic in a provider session is recorded like Python's non-Limina
//!   exceptions (`runtime_error`, then `runtime.failed` with "The managed runtime raised an
//!   unexpected internal error."). Database failures are `LiminaError`s (`database_error`, "The
//!   runtime database is unavailable."), so runs report that code instead of `runtime_error`.
//! - **Shutdown** waits up to 30 s for each loop to record its cancelled turn, then aborts it.
//!   `ensure_running` after shutdown is a `transport_error` (503) rather than an unhandled error.
//! - **Engine readiness** counts an engine as installed when its CLI is on `PATH` (the Rust
//!   engines speak the CLIs' protocols; Python checked that the SDK was importable).
//! - **Disconnecting an unknown engine** is an `invariant_violation` (422) instead of an
//!   unhandled `ValueError` (500).
//! - **Uploads** are stored by [`ProjectOperations::upload_source`] (Python did it in the route).
//! - **The workspace root is absolute**, so provider working directories never depend on the
//!   server's current directory.
//! - **Empty environment variables count as unset** in [`RuntimeConfig::from_env`] (except
//!   `LIMINA_RUNTIME_RETRY_DELAYS_SECONDS`, which is validated as Python did).

pub mod capabilities;
pub mod connections;
pub mod engine_accounts;
pub mod engine_status;
pub mod notifier;
pub mod operations;
pub mod prompt;
pub mod runtime;
pub mod sharing;
pub mod supervisor;
pub mod views;

pub use engine_accounts::{ClaudeOverlay, EngineAccount, ProjectEngineAccounts};
pub use engine_status::{
    CODEX_STATUS_TTL, EngineStatusOverride, EnvironmentSource, process_environment_source,
};
pub use notifier::EventNotifier;
pub use operations::{
    ACTOR_LIMIT, DEFAULT_UPLOAD_LIMIT_BYTES, NewProject, PUBLIC_COMMAND_ID_LIMIT,
    ProjectOperations, RuntimeCodexLogin, Upload, scoped_command_id,
};
pub use prompt::turn_prompt;
pub use runtime::{Runtime, RuntimeConfig};
pub use sharing::PeopleDirectory;
pub use supervisor::{Guidance, ProjectSupervisor, Submitted, SupervisorOptions};
pub use views::{
    ArtifactResponse, Delivery, EventPage, EventResponse, GuidanceReceipt, KickoffTemplate,
    KnowledgeGraphResponse, KnowledgePageResponse, ProjectEngineAccountResponse, ProjectResponse,
    ProjectStatusResponse, ResourceResponse, ReviewResource, ReviewResponse, RuntimeRunDetail,
    SnapshotResponse, TagResponse, TemplateDefaults, public_artifact, public_event, public_project,
    public_resource, public_run_event, public_status,
};
