//! A credential-free runtime for console development and demos: the scripted engine, its Codex
//! sign-in and the seeded demo project.
//!
//! A port of `scripts/ui_dev_runtime.py`. No provider is ever called. A deterministic agent
//! ([`ScriptedAgent`]) drives the real supervisor: it streams message, command, edit and knowledge
//! events, writes hypotheses, experiments and findings through the internal agent API
//! (`/internal/v1/projects/{slug}/…`, with the turn's capability token) exactly as Codex and
//! Claude do, accepts live steering, can be interrupted, and pauses once to ask the human for a
//! decision. Project accounts work too: a pasted Claude token is stored as usual, and the Codex
//! device sign-in ([`ScriptedCodexConnector`]) shows code `DEMO-2468` and confirms itself after a
//! few seconds.
//!
//! The scenario is chosen from the project's artifacts at the start of each turn, so it resumes
//! wherever the project is. It keeps no provider thread, so a real engine can take over a seeded
//! project and start fresh from its knowledge base.
//!
//! # Running it (`limina dev`)
//!
//! ```no_run
//! # async fn dev() -> limina_core::Result<()> {
//! use limina_server::app::Runtime;
//! use limina_server::dev::{self, DevOptions};
//!
//! let options = DevOptions::from_env()?;
//! let runtime = Runtime::new(dev::prepare(&options).await?).await?;
//! dev::seed_demo(&runtime).await?;
//! eprintln!("{}", options.startup_notice());
//! // Serve the HTTP API (with the internal agent routes) on 127.0.0.1:<options.port>, with
//! // `options.api_token`, `options.admin_token` and `options.web_dir`, and call
//! // `runtime.start()` once it listens. In seed mode, stop serving when the demo is seeded:
//! if options.seed_only {
//!     let outcome = dev::wait_until_seeded(&runtime).await?;
//!     eprintln!("{}", outcome.message());
//! }
//! runtime.shutdown().await;
//! # Ok(())
//! # }
//! ```
//!
//! The scripted agent reaches the internal API at [`DevOptions::internal_url`] and retries its
//! first request for about ten seconds, so the server may start listening after the demo
//! project's loop has started.
//!
//! # Environment
//!
//! | Variable | Default | Meaning |
//! |---|---|---|
//! | `LIMINA_DEV_PACE` | `1.2` | seconds between streamed events (the unit every step scales) |
//! | `LIMINA_DEV_DATA` | `.limina/ui-dev` | state directory: database, workspaces, Codex homes, key |
//! | `LIMINA_DEV_DATABASE_URL` | `sqlite:///<data>/runtime.db` | e.g. a deployment's database to seed |
//! | `LIMINA_DEV_SEED` | off | `1`/`true`/`yes`: play the demo until its first question, then exit |
//! | `LIMINA_DEV_PORT` | `7433` | where the runtime listens (and the internal API URL) |
//! | `LIMINA_DEV_SHARED_CREDENTIALS` | on | `0`/`false`: every project must connect its own account |
//! | `LIMINA_API_TOKEN`, `LIMINA_ADMIN_API_TOKEN` | `dev`, `dev-admin` | the console's shared tokens |
//!
//! Everything else (`LIMINA_SECRET_KEY`, retry delays, upload limit, …) is read as
//! [`RuntimeConfig::from_env`](crate::app::RuntimeConfig::from_env) reads it.
//!
//! # Deliberate differences from the Python script
//!
//! - **Interrupts are immediate.** Python noticed an interrupt when the current pause ended; the
//!   pause now ends as soon as the turn is interrupted. The turn still fails with the same
//!   `transport_error`.
//! - **Internal API failures are `transport_error`s** naming the request and the server's message
//!   (`status`, `reason`). Python raised `httpx` exceptions, which the supervisor recorded as an
//!   unexpected `runtime_error`.
//! - **A cancelled scripted Codex sign-in stays signed out.** Python's sign-in wrote `auth.json`
//!   after its delay even when the attempt had been cancelled.
//! - **The runtime's own Codex account is scripted too.** The connector replaces `codex
//!   app-server` for the runtime's shared Codex home as well as project homes (Python only
//!   replaced the project sign-in), so no Codex process ever starts.
//! - **Seeding only creates a missing project.** Python created the demo after any error reading
//!   it; other errors (an unavailable database) are now returned.
//! - **Seeding starts the demo's loop** with
//!   [`ensure_running`](crate::app::ProjectSupervisor::ensure_running), so it works before or
//!   after [`Runtime::start`](crate::app::Runtime::start).
//! - **`LIMINA_DEV_SHARED_CREDENTIALS`** is read case-insensitively, and empty variables count as
//!   unset; invalid numbers are configuration errors instead of a crash.

mod agent;
mod client;
mod codex;
mod options;
mod scenario;
mod setup;

pub use agent::{INTERRUPTED, SCRIPTED_MODEL, STEERING_ACK, ScriptedAgent, scripted_factory};
pub use codex::{
    DEVICE_USER_CODE, DEVICE_VERIFICATION_URL, SCRIPTED_ACCOUNT_EMAIL, SCRIPTED_ACCOUNT_PLAN,
    ScriptedCodexConnector,
};
pub use options::{
    DEFAULT_ADMIN_TOKEN, DEFAULT_API_TOKEN, DEFAULT_DATA, DEFAULT_PACE, DEFAULT_PORT,
    DEFAULT_SIGN_IN_DELAY, DevOptions,
};
pub use setup::{
    DEMO_ACTOR, DEMO_SLUG, NO_SHARED_CREDENTIALS_DETAIL, SCRIPTED_ENGINE_DETAIL, SeedOutcome,
    prepare, scripted_engine_status, seed_demo, wait_until_seeded,
};
