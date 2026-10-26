//! A writer-preferring gate: turns share reads; an auth mutation owns the writer.
//!
//! Unlike a lock guard, the writer can be released from another task (`end_write`), because a
//! device login holds it from the request that starts it until the background wait finishes.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::Notify;

#[derive(Default)]
struct State {
    readers: usize,
    writer: bool,
    writers_waiting: usize,
}

#[derive(Default)]
pub(crate) struct Gate {
    state: Mutex<State>,
    changed: Notify,
}

impl Gate {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn readers(&self) -> usize {
        self.lock().readers
    }

    /// Shares the gate, waiting while a writer is active or queued.
    pub(crate) async fn read(self: &Arc<Self>) -> ReadGuard {
        loop {
            // Registered before the check, so a release in between still wakes this waiter.
            let changed = self.changed.notified();
            {
                let mut state = self.lock();
                if !state.writer && state.writers_waiting == 0 {
                    state.readers += 1;
                    return ReadGuard(Arc::clone(self));
                }
            }
            changed.await;
        }
    }

    /// Shares the gate only if no writer is active or queued; never waits.
    pub(crate) fn try_read(self: &Arc<Self>) -> Option<ReadGuard> {
        let mut state = self.lock();
        if state.writer || state.writers_waiting > 0 {
            return None;
        }
        state.readers += 1;
        Some(ReadGuard(Arc::clone(self)))
    }

    /// Takes the writer, waiting for readers and any other writer to finish. New readers queue
    /// behind a waiting writer.
    pub(crate) async fn write(self: &Arc<Self>) -> WriteGuard {
        self.lock().writers_waiting += 1;
        let waiting = WaitingWriter(self);
        loop {
            let changed = self.changed.notified();
            {
                let mut state = self.lock();
                if !state.writer && state.readers == 0 {
                    state.writer = true;
                    drop(state);
                    drop(waiting);
                    return WriteGuard(Arc::clone(self));
                }
            }
            changed.await;
        }
    }

    /// Takes the writer only if nobody holds the gate; the caller must `end_write`.
    pub(crate) fn try_begin_write(&self) -> bool {
        let mut state = self.lock();
        if state.writer || state.readers > 0 {
            return false;
        }
        state.writer = true;
        true
    }

    /// Releases the writer. Releasing an unheld writer does nothing.
    pub(crate) fn end_write(&self) {
        let mut state = self.lock();
        if !state.writer {
            return;
        }
        state.writer = false;
        drop(state);
        self.changed.notify_waiters();
    }
}

pub(crate) struct ReadGuard(Arc<Gate>);

impl Drop for ReadGuard {
    fn drop(&mut self) {
        self.0.lock().readers -= 1;
        self.0.changed.notify_waiters();
    }
}

pub(crate) struct WriteGuard(Arc<Gate>);

impl Drop for WriteGuard {
    fn drop(&mut self) {
        self.0.end_write();
    }
}

/// Keeps `writers_waiting` right even if a waiting writer is cancelled.
struct WaitingWriter<'a>(&'a Gate);

impl Drop for WaitingWriter<'_> {
    fn drop(&mut self) {
        self.0.lock().writers_waiting -= 1;
        self.0.changed.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn readers_share_and_writers_are_exclusive() {
        let gate = Arc::new(Gate::default());
        let first = gate.read().await;
        let second = gate.read().await;
        assert_eq!(gate.readers(), 2);
        assert!(!gate.try_begin_write());
        drop((first, second));
        assert!(gate.try_begin_write());
        assert!(gate.try_read().is_none());
        gate.end_write();
        gate.end_write();
        assert!(gate.try_read().is_some());
    }

    #[tokio::test]
    async fn a_waiting_writer_blocks_new_readers_and_cancels_cleanly() {
        let gate = Arc::new(Gate::default());
        let reader = gate.read().await;
        let writer = tokio::spawn({
            let gate = Arc::clone(&gate);
            async move {
                let _guard = gate.write().await;
            }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(gate.try_read().is_none(), "a queued writer keeps new readers out");
        let late_reader = tokio::spawn({
            let gate = Arc::clone(&gate);
            async move {
                let _guard = gate.read().await;
            }
        });
        drop(reader);
        writer.await.unwrap();
        late_reader.await.unwrap();

        let reader = gate.read().await;
        let cancelled = tokio::time::timeout(Duration::from_millis(20), gate.write()).await;
        assert!(cancelled.is_err());
        drop(reader);
        assert!(gate.try_read().is_some(), "a cancelled writer no longer blocks readers");
    }
}
