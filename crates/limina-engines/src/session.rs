//! The provider-neutral agent session the supervisor drives.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use limina_core::Result;
use limina_domain::Engine;
use tokio::sync::mpsc;

use crate::events::RuntimeEvent;
use crate::mcp::McpServer;
use crate::turn::RuntimeTurn;

/// Receives what a turn produces while it runs.
///
/// Both callbacks are awaited in order before the session continues, so the supervisor can
/// persist a continuation before any tool runs. An error aborts the turn and is returned from
/// [`AgentSession::run_turn`] unchanged.
#[async_trait]
pub trait TurnObserver: Send + Sync {
    /// A normalized transcript event (see [`crate::events`]).
    async fn event(&self, event: RuntimeEvent) -> Result<()>;

    /// The provider thread or session to resume next time. It may repeat.
    async fn continuation(&self, continuation_id: &str) -> Result<()>;
}

/// Inference defaults snapshotted at the start of a turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelSettings {
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}

/// Everything one turn needs.
#[derive(Clone)]
pub struct TurnRequest {
    pub prompt: String,
    pub settings: ModelSettings,
    /// The project workspace, used as the provider's working directory.
    pub workspace: PathBuf,
    /// The previous turn's continuation, to resume the same provider thread or session.
    pub continuation_id: Option<String>,
    /// Project resources and Limina capabilities added to the provider's isolated environment.
    /// Their values are also redacted from provider error reasons.
    pub runtime_env: BTreeMap<String, String>,
    /// The connected MCP servers the project's owners enabled (see [`crate::mcp`]). Their
    /// credentials are redacted from provider error reasons too.
    pub mcp_servers: Vec<McpServer>,
    pub observer: Arc<dyn TurnObserver>,
}

impl TurnRequest {
    /// Every value a provider error reason must not show: resource values and capabilities,
    /// then MCP credentials.
    pub fn redactions(&self) -> Vec<&String> {
        self.runtime_env
            .values()
            .chain(self.mcp_servers.iter().flat_map(McpServer::secrets))
            .collect()
    }
}

/// One project's long-lived connection to a provider. `run_turn` runs one turn at a time;
/// `steer`, `interrupt` and `close` may be called concurrently from other tasks.
#[async_trait]
pub trait AgentSession: Send + Sync {
    async fn run_turn(&self, request: TurnRequest) -> Result<RuntimeTurn>;

    /// Read current provider model metadata without sending a prompt or altering an active turn.
    async fn models(
        &self,
        _runtime_env: &BTreeMap<String, String>,
    ) -> Result<crate::models::ModelCatalog> {
        Err(limina_core::LiminaError::transport(
            "Model discovery is unavailable for this provider.",
        ))
    }

    /// Delivers human guidance to the active turn. `Ok(false)` when no turn can take it now
    /// (the supervisor then queues it for the next turn).
    async fn steer(&self, message: &str) -> Result<bool>;

    /// Asks the active turn to stop. `Ok(false)` when there is nothing to interrupt.
    async fn interrupt(&self) -> Result<bool>;

    /// Interrupts any active turn and releases the provider process. Never fails.
    async fn close(&self);

    /// The model the session runs, when one is configured.
    fn model(&self) -> Option<String> {
        None
    }
}

/// Creates a project's session for its engine. The supervisor stores one per project.
pub type AgentFactory = Arc<dyn Fn(&str, Engine) -> Result<Arc<dyn AgentSession>> + Send + Sync>;

/// What a [`ChannelObserver`] forwards.
#[derive(Debug, Clone, PartialEq)]
pub enum TurnSignal {
    Event(RuntimeEvent),
    Continuation(String),
}

/// A [`TurnObserver`] that forwards everything to an unbounded channel, in order.
#[derive(Debug, Clone)]
pub struct ChannelObserver {
    sender: mpsc::UnboundedSender<TurnSignal>,
}

impl ChannelObserver {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<TurnSignal>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (Self { sender }, receiver)
    }
}

#[async_trait]
impl TurnObserver for ChannelObserver {
    async fn event(&self, event: RuntimeEvent) -> Result<()> {
        // A dropped receiver means nobody is listening any more; the turn itself goes on.
        let _ = self.sender.send(TurnSignal::Event(event));
        Ok(())
    }

    async fn continuation(&self, continuation_id: &str) -> Result<()> {
        let _ = self.sender.send(TurnSignal::Continuation(continuation_id.to_owned()));
        Ok(())
    }
}
