//! Signing a project in to Claude with the official CLI.
//!
//! `claude auth login --claudeai` prints an authorization URL and waits for the code claude.com
//! shows once the person has signed in with their own account. Limina relays both through the
//! console: it starts the CLI in the project's private Claude configuration directory, returns
//! the URL, and writes the pasted code to the CLI. The credentials stay in that directory, where
//! the CLI refreshes them for the project's turns; Limina never sees them.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use limina_core::{LiminaError, Result, Timestamp, iso, iso_opt, new_uuid, now};
use limina_domain::environment::{claude_environment, process_environment};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;

use crate::codex::auth::DeviceLoginAttempt;

const LOG_TARGET: &str = "limina_engines::claude::login";
/// How long the CLI gets to print its authorization URL.
const URL_WAIT: Duration = Duration::from_secs(30);
/// How long the CLI gets to exchange a pasted code.
const CODE_WAIT: Duration = Duration::from_secs(90);
/// A pending sign-in is abandoned after this long.
const ATTEMPT_LIFETIME: Duration = Duration::from_secs(15 * 60);
/// Finished attempts are kept this long so the console can read their outcome.
const FINISHED_RETENTION: Duration = Duration::from_secs(10 * 60);
const STATUS_WAIT: Duration = Duration::from_secs(20);
const OUTPUT_TAIL_LINES: usize = 40;
const CODE_LIMIT: usize = 4096;
/// Credentials that would override the project's own sign-in inside the CLI.
const COMPETING_ENV: [&str; 6] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
];

/// What `claude auth status` reports for a configuration directory.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeAccountStatus {
    #[serde(default)]
    pub logged_in: bool,
    #[serde(default)]
    pub auth_method: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub subscription_type: Option<String>,
}

impl ClaudeAccountStatus {
    /// `email · plan`, or whichever of them is known.
    pub fn label(&self) -> Option<String> {
        let parts: Vec<&str> = [self.email.as_deref(), self.subscription_type.as_deref()]
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Pending,
    Succeeded,
    Failed,
    Cancelled,
}

impl Status {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
            Self::Cancelled => "CANCELLED",
        }
    }
}

struct State {
    status: Status,
    error: Option<String>,
    completed_at: Option<Timestamp>,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
}

struct Attempt {
    login_id: String,
    project: String,
    command_id: String,
    config_dir: PathBuf,
    verification_url: String,
    created_at: Timestamp,
    state: tokio::sync::Mutex<State>,
    output: Arc<Mutex<VecDeque<String>>>,
}

impl Attempt {
    async fn public(&self) -> DeviceLoginAttempt {
        let state = self.state.lock().await;
        DeviceLoginAttempt {
            login_id: self.login_id.clone(),
            status: state.status.as_str().to_owned(),
            verification_url: self.verification_url.clone(),
            user_code: None,
            error: state.error.clone(),
            created_at: iso(&self.created_at),
            completed_at: iso_opt(state.completed_at.as_ref()),
        }
    }

    async fn finish(&self, status: Status, error: Option<String>) {
        let mut state = self.state.lock().await;
        if state.status != Status::Pending {
            return;
        }
        state.status = status;
        state.error = error;
        state.completed_at = Some(now());
        state.stdin = None;
        if let Some(mut child) = state.child.take() {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
    }

    fn output_tail(&self) -> String {
        let lines = self.output.lock().unwrap_or_else(PoisonError::into_inner);
        lines.iter().cloned().collect::<Vec<_>>().join("\n")
    }
}

/// Claude sign-ins in progress, one CLI process per attempt.
pub struct ClaudeLogins {
    claude_bin: String,
    environment: BTreeMap<String, String>,
    attempts: Mutex<HashMap<String, Arc<Attempt>>>,
}

impl std::fmt::Debug for ClaudeLogins {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeLogins").field("claude_bin", &self.claude_bin).finish_non_exhaustive()
    }
}

impl ClaudeLogins {
    /// Sign-ins run `claude_bin` with the runtime's own environment as the inherited base.
    pub fn new(claude_bin: impl Into<String>) -> Self {
        Self::with_environment(claude_bin, process_environment())
    }

    pub fn with_environment(
        claude_bin: impl Into<String>,
        environment: BTreeMap<String, String>,
    ) -> Self {
        Self { claude_bin: claude_bin.into(), environment, attempts: Mutex::new(HashMap::new()) }
    }

    /// Starts a sign-in into `config_dir` for `project`. The same `command_id` returns the same
    /// attempt; a new one cancels the project's previous pending sign-in.
    pub async fn start(
        &self,
        project: &str,
        config_dir: &Path,
        command_id: &str,
    ) -> Result<DeviceLoginAttempt> {
        self.prune().await;
        let existing: Vec<Arc<Attempt>> =
            self.lock().values().filter(|attempt| attempt.project == project).cloned().collect();
        for attempt in &existing {
            if attempt.command_id == command_id {
                return Ok(attempt.public().await);
            }
        }
        for attempt in existing {
            attempt.finish(Status::Cancelled, None).await;
        }

        let env = claude_environment(&self.environment, &competing_overlay(), config_dir)
            .map_err(|error| login_failed("The project's Claude directory is unusable.", error))?;
        let mut child = Command::new(&self.claude_bin)
            .args(["auth", "login", "--claudeai"])
            .env_clear()
            .envs(&env)
            .current_dir(config_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                login_failed("The Claude CLI could not be started.", error).with(
                    "suggestion",
                    "Install the Claude CLI on the runtime (LIMINA_CLAUDE_BIN).",
                )
            })?;
        let output = Arc::new(Mutex::new(VecDeque::new()));
        let (lines, mut received) = mpsc::unbounded_channel();
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(read_lines(stdout, Arc::clone(&output), lines.clone()));
        }
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(read_lines(stderr, Arc::clone(&output), lines));
        }
        let stdin = child.stdin.take();

        let url = tokio::time::timeout(URL_WAIT, async {
            while let Some(line) = received.recv().await {
                if let Some(url) = authorization_url(&line) {
                    return Some(url);
                }
            }
            None
        })
        .await
        .ok()
        .flatten();
        let Some(verification_url) = url else {
            let _ = child.start_kill();
            let tail = output
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            tracing::warn!(target: LOG_TARGET, output = %tail, "Claude did not print a sign-in URL");
            return Err(LiminaError::transport("Claude did not start a sign-in. Try again.")
                .with_code("claude_login_failed"));
        };
        // Keep draining so the CLI never blocks on a full pipe.
        tokio::spawn(async move { while received.recv().await.is_some() {} });

        let attempt = Arc::new(Attempt {
            login_id: new_uuid(),
            project: project.to_owned(),
            command_id: command_id.to_owned(),
            config_dir: config_dir.to_path_buf(),
            verification_url,
            created_at: now(),
            state: tokio::sync::Mutex::new(State {
                status: Status::Pending,
                error: None,
                completed_at: None,
                child: Some(child),
                stdin,
            }),
            output,
        });
        self.lock().insert(attempt.login_id.clone(), Arc::clone(&attempt));
        let expiring = Arc::clone(&attempt);
        tokio::spawn(async move {
            tokio::time::sleep(ATTEMPT_LIFETIME).await;
            expiring
                .finish(Status::Failed, Some("The sign-in expired. Start a new one.".into()))
                .await;
        });
        Ok(attempt.public().await)
    }

    /// Hands the code claude.com showed to the CLI and waits for it to finish signing in.
    pub async fn submit_code(&self, login_id: &str, code: &str) -> Result<DeviceLoginAttempt> {
        let attempt = self.find(login_id)?;
        let code = code.trim();
        if code.is_empty() || code.len() > CODE_LIMIT || code.contains(['\n', '\r']) {
            return Err(LiminaError::invariant(
                "Paste the code claude.com shows after you sign in.",
            ));
        }
        let mut state = attempt.state.lock().await;
        if state.status != Status::Pending {
            drop(state);
            return Ok(attempt.public().await);
        }
        let Some(mut stdin) = state.stdin.take() else {
            drop(state);
            return Ok(attempt.public().await);
        };
        let written = async {
            stdin.write_all(code.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
            stdin.flush().await
        }
        .await;
        drop(stdin);
        let Some(mut child) = state.child.take() else {
            drop(state);
            return Ok(attempt.public().await);
        };
        drop(state);
        let exited = match written {
            Err(error) => Err(error.to_string()),
            Ok(()) => match tokio::time::timeout(CODE_WAIT, child.wait()).await {
                Ok(Ok(status)) if status.success() => Ok(()),
                Ok(Ok(status)) => Err(format!("exited with {status}")),
                Ok(Err(error)) => Err(error.to_string()),
                Err(_) => {
                    let _ = child.start_kill();
                    Err("timed out".to_owned())
                }
            },
        };
        let signed_in = exited.is_ok() && self.status(&attempt.config_dir).await.logged_in;
        if signed_in {
            attempt.finish(Status::Succeeded, None).await;
        } else {
            tracing::warn!(
                target: LOG_TARGET,
                reason = exited.err().unwrap_or_default(),
                output = %attempt.output_tail(),
                "Claude sign-in did not complete"
            );
            attempt
                .finish(
                    Status::Failed,
                    Some(
                        "Claude didn't accept that code. Start a new sign-in and paste the code \
                         exactly as claude.com shows it."
                            .to_owned(),
                    ),
                )
                .await;
        }
        Ok(attempt.public().await)
    }

    pub async fn attempt(&self, login_id: &str) -> Result<DeviceLoginAttempt> {
        Ok(self.find(login_id)?.public().await)
    }

    /// The project an attempt signs in, so callers can check it belongs to the right project.
    pub fn project_of(&self, login_id: &str) -> Result<String> {
        Ok(self.find(login_id)?.project.clone())
    }

    pub async fn cancel(&self, login_id: &str) -> Result<DeviceLoginAttempt> {
        let attempt = self.find(login_id)?;
        attempt.finish(Status::Cancelled, None).await;
        Ok(attempt.public().await)
    }

    /// The account signed in to `config_dir` (signed out when the CLI cannot tell).
    pub async fn status(&self, config_dir: &Path) -> ClaudeAccountStatus {
        let output = self.run(config_dir, &["auth", "status", "--json"]).await;
        output.and_then(|text| serde_json::from_str(text.trim()).ok()).unwrap_or_default()
    }

    /// Signs `config_dir` out and removes the stored credentials even if the CLI fails.
    pub async fn logout(&self, config_dir: &Path) {
        if config_dir.is_dir() {
            let _ = self.run(config_dir, &["auth", "logout"]).await;
            let _ = tokio::fs::remove_file(config_dir.join(".credentials.json")).await;
        }
    }

    /// Cancels every pending sign-in.
    pub async fn close(&self) {
        let attempts: Vec<Arc<Attempt>> = self.lock().values().cloned().collect();
        for attempt in attempts {
            attempt.finish(Status::Cancelled, None).await;
        }
    }

    async fn run(&self, config_dir: &Path, args: &[&str]) -> Option<String> {
        let env = claude_environment(&self.environment, &competing_overlay(), config_dir).ok()?;
        let output = Command::new(&self.claude_bin)
            .args(args)
            .env_clear()
            .envs(&env)
            .current_dir(config_dir)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output();
        match tokio::time::timeout(STATUS_WAIT, output).await {
            Ok(Ok(output)) if output.status.success() => {
                Some(String::from_utf8_lossy(&output.stdout).into_owned())
            }
            Ok(Ok(output)) => {
                tracing::debug!(target: LOG_TARGET, ?args, status = %output.status, "Claude CLI failed");
                None
            }
            Ok(Err(error)) => {
                tracing::warn!(target: LOG_TARGET, ?args, %error, "Claude CLI could not run");
                None
            }
            Err(_) => None,
        }
    }

    fn find(&self, login_id: &str) -> Result<Arc<Attempt>> {
        self.lock().get(login_id).cloned().ok_or_else(|| {
            LiminaError::not_found("That Claude sign-in does not exist or has expired.")
                .with("login_id", login_id)
        })
    }

    async fn prune(&self) {
        let attempts: Vec<Arc<Attempt>> = self.lock().values().cloned().collect();
        let cutoff = now() - chrono::Duration::from_std(FINISHED_RETENTION).unwrap_or_default();
        for attempt in attempts {
            let finished = attempt.state.lock().await.completed_at;
            if finished.is_some_and(|at| at < cutoff) {
                self.lock().remove(&attempt.login_id);
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Attempt>>> {
        self.attempts.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn competing_overlay() -> BTreeMap<String, String> {
    COMPETING_ENV.iter().map(|name| ((*name).to_owned(), String::new())).collect()
}

fn login_failed(message: &str, error: impl std::fmt::Display) -> LiminaError {
    LiminaError::transport(message)
        .with_code("claude_login_failed")
        .with("reason", error.to_string())
}

/// The first `https://…oauth…` URL in a line of CLI output.
fn authorization_url(line: &str) -> Option<String> {
    line.split_whitespace()
        .find(|word| word.starts_with("https://") && word.contains("oauth"))
        .map(|word| word.trim_end_matches(['.', ',', ')', '"', '\'']).to_owned())
}

async fn read_lines(
    stream: impl AsyncRead + Unpin,
    output: Arc<Mutex<VecDeque<String>>>,
    lines: mpsc::UnboundedSender<String>,
) {
    let mut reader = BufReader::new(stream).lines();
    while let Ok(Some(line)) = reader.next_line().await {
        {
            let mut tail = output.lock().unwrap_or_else(PoisonError::into_inner);
            if tail.len() == OUTPUT_TAIL_LINES {
                tail.pop_front();
            }
            // The URL carries a one-time PKCE state; keep logs readable without it.
            tail.push_back(if authorization_url(&line).is_some() {
                "[authorization URL]".to_owned()
            } else {
                line.clone()
            });
        }
        if lines.send(line).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_authorization_url_is_found_in_the_cli_output() {
        let line = "If the browser didn't open, visit: https://claude.com/cai/oauth/authorize?code=true&client_id=x";
        assert_eq!(
            authorization_url(line).as_deref(),
            Some("https://claude.com/cai/oauth/authorize?code=true&client_id=x")
        );
        assert_eq!(authorization_url("Opening browser to sign in…"), None);
    }

    #[test]
    fn accounts_are_labelled_with_email_and_plan() {
        let status: ClaudeAccountStatus = serde_json::from_str(
            r#"{"loggedIn": true, "authMethod": "claude.ai", "email": "ada@example.com", "subscriptionType": "max", "orgName": "x"}"#,
        )
        .unwrap();
        assert!(status.logged_in);
        assert_eq!(status.label().as_deref(), Some("ada@example.com · max"));
        assert_eq!(ClaudeAccountStatus::default().label(), None);
    }
}
