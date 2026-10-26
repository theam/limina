//! Codex and Claude provider sessions for the Limina runtime.
//!
//! There is no official Rust SDK for either provider, so this crate speaks their protocols
//! directly, the way the official Python SDKs the Python runtime used do:
//!
//! - **Codex**: `codex app-server` over stdio JSON-RPC ([`codex::rpc`]), mirroring
//!   `openai_codex` 0.1.0b2. One app-server per turn; threads resume across turns; turns can be
//!   steered and interrupted; account reads and logins run through [`codex::auth`].
//! - **Claude**: the `claude` CLI in `stream-json` mode with its control protocol
//!   ([`claude::session`]), mirroring `claude_agent_sdk` 0.2.118: Limina answers `can_use_tool`
//!   requests, interrupts to steer, and resumes the project's session.
//!
//! Both implement [`AgentSession`], which the supervisor drives through an [`AgentFactory`];
//! tests and other runtimes can inject their own sessions. Provider output becomes
//! conversation-grade [`RuntimeEvent`]s ([`events`]), checkpoints become [`RuntimeTurn`]s, and
//! token usage becomes `limina_domain::usage::RuntimeUsage` ([`usage`]).
//!
//! Provider processes start from an empty environment plus the policy maps in
//! `limina_domain::environment`, are read concurrently on stdout and stderr, and are killed
//! when their session lets go of them.

pub mod availability;
pub mod claude;
pub mod codex;
pub mod error;
pub mod events;
pub mod factory;
pub mod mcp;
pub mod models;
mod process;
pub mod redaction;
pub mod session;
pub mod settings;
mod text;
pub mod turn;
pub mod usage;

pub use availability::{
    EngineAvailability, binary_installed, claude_availability, codex_availability,
};
pub use claude::login::{ClaudeAccountStatus, ClaudeLogins};
pub use claude::session::{ClaudeAgentSession, ClaudeSessionConfig};
pub use codex::accounts::{
    CodexAuthResolver, CodexAuthRouter, ProjectCodexAccounts, ProjectCodexAuth,
};
pub use codex::auth::{
    CodexAuthManager, CodexAuthOptions, CodexAuthStatus, CodexTurnGuard, DeviceLoginAttempt,
};
pub use codex::session::{CodexAgentSession, CodexSessionConfig};
pub use error::ProviderError;
pub use events::{ClaudeToolNames, RuntimeEvent, map_claude_message, map_codex_notification};
pub use factory::DefaultAgentFactory;
pub use mcp::McpServer;
pub use redaction::{redact_event, redact_text, redact_turn, redact_value};
pub use session::{
    AgentFactory, AgentSession, ChannelObserver, ModelSettings, TurnObserver, TurnRequest,
    TurnSignal,
};
pub use settings::EngineSettings;
pub use turn::{RuntimeDecision, RuntimeTurn, parse_runtime_decision, turn_output_schema};
