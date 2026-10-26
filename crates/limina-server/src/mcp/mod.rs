//! The Model Context Protocol server: Limina's public project contract for agents.
//!
//! Built on the official Rust SDK (`rmcp`). It serves 26
//! tools and 5 resources over Streamable HTTP, statelessly and with JSON responses, and every
//! call goes through [`ProjectOperations`] as the authenticated caller, so MCP and REST share one
//! set of rules, receipts and audit history.
//!
//! # Mounting
//!
//! [`router`] serves MCP at its own root. The HTTP layer mounts it under `/mcp` with
//! `nest_service`, which routes both `/mcp` and `/mcp/` (Python's canonical URL) to it; axum's
//! `nest` would not match `/mcp/`:
//!
//! ```no_run
//! # async fn serve(runtime: limina_server::app::Runtime, auth: limina_server::auth::Auth) {
//! use limina_server::mcp::{self, McpOptions};
//!
//! let mcp = mcp::router(&runtime, auth, McpOptions::from_env());
//! let app: axum::Router = axum::Router::new().nest_service("/mcp", mcp);
//! # }
//! ```
//!
//! Other paths under `/mcp/` are 404s. Python redirected `/mcp` to `/mcp/` (307); both are served
//! directly here. The router needs nothing from its parent: no state, no connect info (MCP
//! failures count against the global authentication ceiling only, as in Python) and no body
//! limit (the transport refuses bodies over 4 MiB with 413). It may sit under
//! [`crate::auth::Auth::layer`], which stays inert for routes that do not ask for a caller.
//!
//! # The boundary
//!
//! See [`security`]: bearer authentication through [`crate::auth::Auth::transport`] (401, 403
//! and 429 as the API's JSON errors), then DNS rebinding protection on `Host` (421) and `Origin`
//! (403), configured by [`McpOptions`]. Browser CORS, when configured, is the HTTP layer's.
//!
//! | Variable | Default | Meaning |
//! |---|---|---|
//! | `LIMINA_MCP_ALLOWED_HOSTS` | `127.0.0.1:*,localhost:*,[::1]:*` | accepted `Host` values |
//! | `LIMINA_MCP_ALLOWED_ORIGINS` | `http://127.0.0.1:*,http://localhost:*,http://[::1]:*` | accepted `Origin` values |
//!
//! The transport is the SDK's, configured like FastMCP's `stateless_http=True,
//! json_response=True`: no sessions, one JSON response per request, `202` for notifications.
//! Clients that accept only `application/json` are served, as Python served them.
//!
//! # Tools and resources
//!
//! [`tools::TOOLS`] lists the tools with the Python server's names, descriptions, JSON Schemas
//! (argument names, types, defaults and bounds) and annotations; `tests/fixtures/mcp_shapes.json`
//! is the reference generated from the Python server. Static-token callers may name the teammate
//! a write is recorded for with the `actor` argument; writes take an optional `idempotency_key`
//! (a fresh `mcp:<uuid>` otherwise), scoped to the caller like REST idempotency keys.
//!
//! Secret values are never tool arguments: they would enter model transcripts. Secrets are set
//! through the trusted CLI or REST API; MCP can list their redacted metadata and revoke them.
//!
//! Results are the Python tools' dicts (see `results.rs`): the structured content, plus the same
//! data as indented JSON text (one block per item for list results).
//!
//! Failures inside a tool are error results (`isError: true`) the model can read, with the text
//! Python's clients received: `Error executing tool <name>: <message>`, where the message is the
//! operation's (`Challenge 'x' does not exist.`) or Pydantic's argument report; unknown tools
//! read `Unknown tool: <name>`. (Python raised `INVALID_PARAMS`/`INTERNAL_ERROR` protocol
//! errors from its tools, but FastMCP turned every exception raised in a tool into this result;
//! the wire behavior is what is kept.) Resource reads fail with that mapping as JSON-RPC errors:
//! `-32602` (invalid params) for caller errors (HTTP status below 500), `-32603` (internal) for
//! runtime failures, the operation's message and `{"code", "details"}` as data; `-32002` for
//! unknown URIs.
//!
//! # Deliberate differences from the Python server
//!
//! - **Authentication failures** other than 401 are JSON errors with their own status (403
//!   `permission_denied`, 429 `rate_limited` with `Retry-After`); Python answered them with 500.
//!   The `Bearer` scheme is case-insensitive, as on REST (Python's MCP matched `Bearer ` exactly).
//! - **Resource errors** use the codes above. Python's SDK sent code `0` with the message
//!   wrapped in `Error creating resource from template: …`.
//! - **Argument errors** keep Pydantic's summary, field and message, without echoing the input
//!   value or linking Pydantic's documentation.
//! - **`GET`** answers 405 (no server-initiated stream in stateless mode); Python held an empty
//!   event stream open. `DELETE` is 405 in both.
//! - **`serverInfo.version`** is the Limina version; Python reported its MCP SDK's version.
//! - **Protocol versions** are the Python server's (`2024-11-05` to `2025-11-25`), negotiated
//!   by the official Rust SDK instead of the Python one.

mod results;
pub mod security;
mod server;
pub mod tools;

use std::sync::Arc;

use axum::Router;
use axum::middleware::from_fn_with_state;
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};

pub use security::{DEFAULT_ALLOWED_HOSTS, DEFAULT_ALLOWED_ORIGINS, McpOptions};

use crate::app::{ProjectOperations, Runtime};
use crate::auth::Auth;
use security::Guard;
use server::LiminaMcp;

/// What the server tells clients about itself (`instructions` in the `initialize` result).
pub const INSTRUCTIONS: &str = "Operate durable Limina projects at the mission boundary. Review \
    work and knowledge, steer strategy, provide project variables, and manage project lifecycle. \
    Limina owns provider sessions, threads, subagents, workspaces, and execution recovery. Never \
    ask the user to manage those internals. Secret values must be configured through the trusted \
    Limina CLI or REST API, not through model-visible MCP arguments.";

/// The MCP endpoint over the runtime's operations, at the router's root (see the module docs).
pub fn router(runtime: &Runtime, auth: Auth, options: McpOptions) -> Router {
    operations_router(runtime.operations().clone(), auth, options)
}

/// [`router`] for an operations facade.
pub fn operations_router(operations: ProjectOperations, auth: Auth, options: McpOptions) -> Router {
    let handler = LiminaMcp::new(operations);
    // Stateless JSON responses, like FastMCP's `stateless_http=True, json_response=True`. Host
    // and Origin are checked by the guard with the Python SDK's rules, so the SDK's own checks
    // (which reject `host:*` patterns) are off.
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        .disable_allowed_hosts()
        .disable_allowed_origins();
    let service = StreamableHttpService::new(
        move || Ok(handler.clone()),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    Router::new()
        .route_service("/", service)
        .layer(from_fn_with_state(Guard { auth, options }, security::guard))
}
