//! Provider child processes.
//!
//! A provider starts from an empty environment (`env_clear()` plus the policy maps from
//! `limina_domain::environment`), with stdin, stdout and stderr piped. Stdout belongs to the
//! protocol reader; stderr is drained concurrently into the log and a bounded tail used in error
//! reasons. The child is killed when the handle is dropped, and `shutdown` closes stdin, waits a
//! grace period for the provider to flush its state, then kills it.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{ChildStdin, ChildStdout, Command};
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;

const LOG_TARGET: &str = "limina_engines::provider";
/// The longest protocol line kept. Longer lines are logged and skipped.
pub(crate) const MAX_LINE_BYTES: usize = 64 * 1024 * 1024;
const STDERR_TAIL_LINES: usize = 400;
/// How long a reader waits for stderr to close before quoting it in an exit reason.
pub(crate) const STDERR_DRAIN: Duration = Duration::from_millis(500);
const READ_CHUNK_BYTES: usize = 64 * 1024;
const KILL_WAIT: Duration = Duration::from_secs(5);
const STDIN_LOCK_WAIT: Duration = Duration::from_secs(5);

/// Aborts a background task when dropped.
pub(crate) struct AbortOnDrop(JoinHandle<()>);

impl AbortOnDrop {
    pub(crate) fn spawn<F>(future: F) -> Self
    where
        F: Future<Output = ()> + Send + 'static,
    {
        Self(tokio::spawn(future))
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// How to start a provider.
pub(crate) struct ProcessSpec<'a> {
    pub label: &'static str,
    pub program: &'a str,
    pub args: &'a [String],
    pub env: &'a BTreeMap<String, String>,
    pub cwd: Option<&'a Path>,
}

#[derive(Default)]
struct StderrTail {
    lines: Mutex<VecDeque<String>>,
}

impl StderrTail {
    fn push(&self, line: String) {
        let mut lines = self.lines.lock().unwrap_or_else(PoisonError::into_inner);
        if lines.len() == STDERR_TAIL_LINES {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    fn tail(&self, limit: usize) -> String {
        let lines = self.lines.lock().unwrap_or_else(PoisonError::into_inner);
        let skip = lines.len().saturating_sub(limit);
        lines.iter().skip(skip).cloned().collect::<Vec<_>>().join("\n")
    }
}

/// A running provider process.
pub(crate) struct ProviderProcess {
    stdin: tokio::sync::Mutex<Option<ChildStdin>>,
    /// `None` while running; then `Some(exit code)` (`None` inside when killed by a signal).
    exit: watch::Receiver<Option<Option<i32>>>,
    kill: Arc<Notify>,
    stderr: Arc<StderrTail>,
    /// Becomes true once stderr reaches end of file.
    stderr_closed: watch::Receiver<bool>,
    _reaper: AbortOnDrop,
    _stderr_reader: AbortOnDrop,
}

impl ProviderProcess {
    /// Starts the provider. Returns its stdout, which the caller's protocol reader owns.
    pub(crate) fn spawn(spec: &ProcessSpec<'_>) -> io::Result<(Self, ChildStdout)> {
        let mut command = Command::new(spec.program);
        command
            .args(spec.args)
            .env_clear()
            .envs(spec.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(cwd) = spec.cwd {
            command.current_dir(cwd);
        }
        let mut child = command.spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or_else(|| io::Error::other("stdout is not piped"))?;
        let stderr_pipe = child.stderr.take();

        let (exit_sender, exit) = watch::channel(None);
        let kill = Arc::new(Notify::new());
        let reaper = AbortOnDrop::spawn({
            let kill = Arc::clone(&kill);
            async move {
                let status = tokio::select! {
                    status = child.wait() => status,
                    () = kill.notified() => {
                        let _ = child.start_kill();
                        child.wait().await
                    }
                };
                let _ = exit_sender.send(Some(status.ok().and_then(|status| status.code())));
            }
        });

        let stderr = Arc::new(StderrTail::default());
        let label = spec.label;
        let (closed_sender, stderr_closed) = watch::channel(false);
        let stderr_reader = AbortOnDrop::spawn({
            let stderr = Arc::clone(&stderr);
            async move {
                if let Some(pipe) = stderr_pipe {
                    let mut lines = LineReader::new(pipe, MAX_LINE_BYTES);
                    while let Some(line) = lines.next_line().await {
                        let line = line.trim_end().to_owned();
                        if line.is_empty() {
                            continue;
                        }
                        tracing::debug!(target: LOG_TARGET, provider = label, "{line}");
                        stderr.push(line);
                    }
                }
                let _ = closed_sender.send(true);
            }
        });

        let process = Self {
            stdin: tokio::sync::Mutex::new(stdin),
            exit,
            kill,
            stderr,
            stderr_closed,
            _reaper: reaper,
            _stderr_reader: stderr_reader,
        };
        Ok((process, stdout))
    }

    /// Writes one protocol line (a newline is appended).
    pub(crate) async fn write_line(&self, line: &str) -> io::Result<()> {
        let mut stdin = self.stdin.lock().await;
        let pipe = stdin
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "stdin is closed"))?;
        let mut bytes = Vec::with_capacity(line.len() + 1);
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
        pipe.write_all(&bytes).await?;
        pipe.flush().await
    }

    /// Closes stdin, the providers' graceful end-of-session signal.
    pub(crate) async fn close_stdin(&self) {
        // A writer stuck on a full pipe must not keep the provider alive.
        if let Ok(mut stdin) = tokio::time::timeout(STDIN_LOCK_WAIT, self.stdin.lock()).await {
            stdin.take();
        }
    }

    /// Waits for the process to exit. Returns its exit code, `None` when killed by a signal.
    pub(crate) async fn wait_exit(&self) -> Option<i32> {
        let mut exit = self.exit.clone();
        match exit.wait_for(Option::is_some).await {
            Ok(status) => (*status).flatten(),
            Err(_) => None,
        }
    }

    /// Kills the process without waiting.
    pub(crate) fn kill(&self) {
        self.kill.notify_one();
    }

    /// Closes stdin, waits `grace` for a clean exit, then kills the process.
    pub(crate) async fn shutdown(&self, grace: Duration) {
        self.close_stdin().await;
        if tokio::time::timeout(grace, self.wait_exit()).await.is_err() {
            self.kill();
            let _ = tokio::time::timeout(KILL_WAIT, self.wait_exit()).await;
        }
    }

    /// Waits (at most `limit`) for stderr to reach end of file, so an exit's last words are in
    /// the tail before it is read.
    pub(crate) async fn stderr_drained(&self, limit: Duration) {
        let mut closed = self.stderr_closed.clone();
        let _ = tokio::time::timeout(limit, closed.wait_for(|closed| *closed)).await;
    }

    /// The last `lines` stderr lines, newest last.
    pub(crate) fn stderr_tail(&self, lines: usize) -> String {
        self.stderr.tail(lines)
    }
}

/// Splits a byte stream into lines, without a trailing newline and decoded lossily. Lines longer
/// than the limit are logged and skipped instead of growing memory without bound.
pub(crate) struct LineReader<R> {
    reader: R,
    buffer: Vec<u8>,
    start: usize,
    scanned: usize,
    chunk: Box<[u8]>,
    max: usize,
    discarding: bool,
    eof: bool,
}

impl<R: AsyncRead + Unpin> LineReader<R> {
    pub(crate) fn new(reader: R, max: usize) -> Self {
        Self {
            reader,
            buffer: Vec::new(),
            start: 0,
            scanned: 0,
            chunk: vec![0; READ_CHUNK_BYTES].into_boxed_slice(),
            max,
            discarding: false,
            eof: false,
        }
    }

    /// The next line, or `None` at end of stream (a final unterminated line is still returned).
    pub(crate) async fn next_line(&mut self) -> Option<String> {
        loop {
            let unscanned = &self.buffer[self.scanned..];
            if let Some(offset) = unscanned.iter().position(|byte| *byte == b'\n') {
                let end = self.scanned + offset;
                let oversized = end - self.start > self.max;
                let line = (!oversized)
                    .then(|| String::from_utf8_lossy(&self.buffer[self.start..end]).into_owned());
                self.start = end + 1;
                self.scanned = self.start;
                self.compact();
                if std::mem::take(&mut self.discarding) {
                    continue;
                }
                match line {
                    Some(line) => return Some(line),
                    None => {
                        warn_oversized(self.max);
                        continue;
                    }
                }
            }
            self.scanned = self.buffer.len();
            if self.buffer.len() - self.start > self.max {
                warn_oversized(self.max);
                self.buffer.clear();
                self.start = 0;
                self.scanned = 0;
                self.discarding = true;
            }
            if self.eof {
                let rest = &self.buffer[self.start..];
                if rest.is_empty() || self.discarding {
                    return None;
                }
                let line = String::from_utf8_lossy(rest).into_owned();
                self.buffer.clear();
                self.start = 0;
                self.scanned = 0;
                return Some(line);
            }
            match self.reader.read(&mut self.chunk).await {
                Ok(0) => self.eof = true,
                Ok(read) => self.buffer.extend_from_slice(&self.chunk[..read]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    tracing::debug!(target: LOG_TARGET, %error, "provider stream failed");
                    self.eof = true;
                }
            }
        }
    }

    fn compact(&mut self) {
        if self.start == self.buffer.len() {
            self.buffer.clear();
            self.start = 0;
            self.scanned = 0;
        } else if self.start > READ_CHUNK_BYTES {
            self.buffer.drain(..self.start);
            self.scanned -= self.start;
            self.start = 0;
        }
    }
}

/// Explains a failed spawn when the working directory is the cause; `None` otherwise.
pub(crate) async fn spawn_failure(cwd: Option<&Path>, error: &io::Error) -> Option<String> {
    let cwd = cwd?;
    let missing = error.kind() == io::ErrorKind::NotFound
        && !tokio::fs::try_exists(cwd).await.unwrap_or(false);
    missing.then(|| format!("Working directory does not exist: {}", cwd.display()))
}

fn warn_oversized(limit: usize) {
    tracing::warn!(target: LOG_TARGET, limit, "skipping an oversized provider line");
}

/// Runs blocking filesystem work off the async runtime.
pub(crate) async fn blocking<T, F>(work: F) -> io::Result<T>
where
    F: FnOnce() -> io::Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work).await.map_err(io::Error::other)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lines_are_split_bounded_and_flushed_at_eof() {
        let input: &[u8] = b"one\r\ntwo\n\nxxxxxxxxxxxxxxxxxxxx\nthree\nfour";
        let mut reader = LineReader::new(input, 10);
        let mut lines = Vec::new();
        while let Some(line) = reader.next_line().await {
            lines.push(line);
        }
        assert_eq!(lines, ["one\r", "two", "", "three", "four"]);
    }

    #[tokio::test]
    async fn invalid_utf8_is_decoded_lossily() {
        let input: &[u8] = b"ok \xff\n";
        let mut reader = LineReader::new(input, 100);
        assert_eq!(reader.next_line().await.as_deref(), Some("ok \u{fffd}"));
        assert_eq!(reader.next_line().await, None);
    }
}
