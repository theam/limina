//! A per-project "new events may exist" signal, so live clients can push without polling.
//!
//! The supervisor and [`super::ProjectOperations`] bump a project's signal after every write that
//! records events. The value is a generation counter: it only says that something changed, never
//! which events. Readers keep their own event cursor and read the log after it.
//!
//! Writes made by other processes (a second runtime node, the CLI in embedded mode) never reach
//! this signal, so subscribers keep a slow periodic poll as a fallback:
//!
//! ```no_run
//! # async fn follow(runtime: &limina_server::app::Runtime) -> limina_core::Result<()> {
//! use std::time::Duration;
//!
//! let slug = "retrieval-quality";
//! // Subscribe before the first read, so no write between the two is missed.
//! let mut changes = runtime.notifier().subscribe(slug);
//! let mut cursor = 0;
//! loop {
//!     for event in runtime.service().events(slug, cursor, 200).await? {
//!         cursor = i64::from(event.sequence);
//!         if let Some(public) = limina_server::app::public_event(&event) {
//!             // send `public` to the client
//!             let _ = public;
//!         }
//!     }
//!     tokio::select! {
//!         changed = changes.changed() => {
//!             if changed.is_err() {
//!                 break; // the runtime is gone
//!             }
//!         }
//!         () = tokio::time::sleep(Duration::from_secs(1)) => {} // other processes' writes
//!     }
//! }
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::watch;

/// One `watch` channel per project, keyed by the lower-cased slug.
#[derive(Debug, Clone, Default)]
pub struct EventNotifier {
    channels: Arc<Mutex<HashMap<String, watch::Sender<u64>>>>,
}

impl EventNotifier {
    pub fn new() -> Self {
        Self::default()
    }

    /// A receiver for the project's signal. Its current value counts as seen, so `changed()`
    /// resolves at the next write.
    pub fn subscribe(&self, slug: &str) -> watch::Receiver<u64> {
        self.channels().entry(key(slug)).or_insert_with(|| watch::Sender::new(0)).subscribe()
    }

    /// Wakes every subscriber of the project. Never blocks and never fails.
    pub fn notify(&self, slug: &str) {
        self.channels()
            .entry(key(slug))
            .or_insert_with(|| watch::Sender::new(0))
            .send_modify(|generation| *generation = generation.wrapping_add(1));
    }

    fn channels(&self) -> MutexGuard<'_, HashMap<String, watch::Sender<u64>>> {
        self.channels.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn key(slug: &str) -> String {
    slug.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscribers_wake_on_their_project_only() {
        let notifier = EventNotifier::new();
        let mut first = notifier.subscribe("Alpha");
        let mut other = notifier.subscribe("beta");
        notifier.notify("alpha");
        first.changed().await.unwrap();
        assert_eq!(*first.borrow_and_update(), 1);
        assert!(!other.has_changed().unwrap());
        notifier.notify("beta");
        other.changed().await.unwrap();
    }

    #[test]
    fn notifying_without_subscribers_is_harmless() {
        let notifier = EventNotifier::new();
        notifier.notify("alpha");
        notifier.notify("alpha");
        assert_eq!(*notifier.subscribe("alpha").borrow(), 2);
    }
}
