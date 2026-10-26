//! Helpers shared by the end-to-end tests.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use limina_core::Result;
use limina_engines::{RuntimeEvent, TurnObserver};
use serde_json::Value;

/// The scripted provider (`tests/support/fake_provider.rs`).
pub fn fake_provider() -> String {
    env!("CARGO_BIN_EXE_limina-fake-provider").to_owned()
}

/// Collects what a turn reports. Optionally signals when the first event arrives.
#[derive(Default)]
pub struct Recorder {
    pub events: Mutex<Vec<RuntimeEvent>>,
    pub continuations: Mutex<Vec<String>>,
    first_event: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl Recorder {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Resolves once the first transcript event has been recorded.
    pub fn first_event(self: &Arc<Self>) -> tokio::sync::oneshot::Receiver<()> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        *self.first_event.lock().unwrap() = Some(sender);
        receiver
    }

    pub fn events(&self) -> Vec<RuntimeEvent> {
        self.events.lock().unwrap().clone()
    }

    pub fn continuations(&self) -> Vec<String> {
        self.continuations.lock().unwrap().clone()
    }

    pub fn kinds(&self) -> Vec<String> {
        self.events()
            .iter()
            .map(|event| event.detail_str("kind").unwrap_or_default().to_owned())
            .collect()
    }
}

#[async_trait]
impl TurnObserver for Recorder {
    async fn event(&self, event: RuntimeEvent) -> Result<()> {
        self.events.lock().unwrap().push(event);
        if let Some(sender) = self.first_event.lock().unwrap().take() {
            let _ = sender.send(());
        }
        Ok(())
    }

    async fn continuation(&self, continuation_id: &str) -> Result<()> {
        self.continuations.lock().unwrap().push(continuation_id.to_owned());
        Ok(())
    }
}

/// The JSON lines a fake provider logged.
pub fn read_log(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Every `start` entry (one per process launch).
pub fn starts(log: &[Value]) -> Vec<Value> {
    log.iter().filter_map(|entry| entry.get("start").cloned()).collect()
}

/// Every message the provider received.
pub fn received(log: &[Value]) -> Vec<Value> {
    log.iter().filter_map(|entry| entry.get("received").cloned()).collect()
}

/// Received JSON-RPC requests with this method.
pub fn requests(log: &[Value], method: &str) -> Vec<Value> {
    received(log)
        .into_iter()
        .filter(|message| message.get("method").and_then(Value::as_str) == Some(method))
        .collect()
}

/// True while a process with this pid exists.
pub fn process_alive(pid: u64) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Polls `condition` for up to five seconds.
pub async fn eventually(mut condition: impl FnMut() -> bool) -> bool {
    for _ in 0..250 {
        if condition() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    condition()
}
