//! The scripted Codex sign-in: stands in for `codex app-server` in account reads and logins.
//!
//! A Codex home is signed in when it holds `auth.json`. A device login shows code
//! [`DEVICE_USER_CODE`] and confirms itself after the connector's delay by writing `auth.json`;
//! the account is then `dev@example.com` on the `plus` plan.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use limina_engines::ProviderError;
use limina_engines::codex::auth::{
    CodexAccount, CodexAuthClient, CodexAuthConnector, DeviceCode, LoginCompletion,
};
use tokio::sync::watch;

use super::client::hex_id;
use super::options::DEFAULT_SIGN_IN_DELAY;

/// Where the scripted device sign-in sends people.
pub const DEVICE_VERIFICATION_URL: &str = "https://example.com/device";
/// The code the scripted device sign-in shows.
pub const DEVICE_USER_CODE: &str = "DEMO-2468";
/// The signed-in account (`dev@example.com · plus`).
pub const SCRIPTED_ACCOUNT_EMAIL: &str = "dev@example.com";
pub const SCRIPTED_ACCOUNT_PLAN: &str = "plus";
const AUTH_FILE: &str = "auth.json";

/// A [`CodexAuthConnector`] whose sign-ins confirm themselves (see the module docs).
#[derive(Debug, Clone)]
pub struct ScriptedCodexConnector {
    delay: Duration,
}

impl ScriptedCodexConnector {
    /// Device logins confirm after [`DEFAULT_SIGN_IN_DELAY`].
    pub fn new() -> Self {
        Self::with_delay(DEFAULT_SIGN_IN_DELAY)
    }

    /// Device logins confirm after `delay`.
    pub fn with_delay(delay: Duration) -> Self {
        Self { delay }
    }
}

impl Default for ScriptedCodexConnector {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CodexAuthConnector for ScriptedCodexConnector {
    async fn connect(
        &self,
        environment: BTreeMap<String, String>,
    ) -> std::result::Result<Arc<dyn CodexAuthClient>, ProviderError> {
        let home = environment
            .get("CODEX_HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| ProviderError::Spawn("CODEX_HOME is not set.".to_owned()))?;
        Ok(Arc::new(ScriptedCodexClient {
            home,
            delay: self.delay,
            stopped: watch::Sender::new(false),
        }))
    }
}

struct ScriptedCodexClient {
    home: PathBuf,
    delay: Duration,
    /// Set by a cancellation or by closing the connection; a pending login then fails.
    stopped: watch::Sender<bool>,
}

fn io_error(error: std::io::Error) -> ProviderError {
    ProviderError::Protocol(format!(
        "The scripted Codex sign-in could not update its home: {error}"
    ))
}

#[async_trait]
impl CodexAuthClient for ScriptedCodexClient {
    async fn account(&self) -> std::result::Result<Option<CodexAccount>, ProviderError> {
        let signed_in = tokio::fs::try_exists(self.home.join(AUTH_FILE)).await.unwrap_or(false);
        Ok(signed_in.then(|| CodexAccount {
            account_type: "chatgpt".to_owned(),
            email: Some(SCRIPTED_ACCOUNT_EMAIL.to_owned()),
            plan_type: Some(SCRIPTED_ACCOUNT_PLAN.to_owned()),
        }))
    }

    async fn login_api_key(&self, _api_key: &str) -> std::result::Result<(), ProviderError> {
        Ok(())
    }

    async fn login_chatgpt_device_code(&self) -> std::result::Result<DeviceCode, ProviderError> {
        Ok(DeviceCode {
            login_id: format!("dev-{}", &hex_id()[..8]),
            verification_url: DEVICE_VERIFICATION_URL.to_owned(),
            user_code: DEVICE_USER_CODE.to_owned(),
        })
    }

    async fn wait_login(
        &self,
        _login_id: &str,
    ) -> std::result::Result<LoginCompletion, ProviderError> {
        let mut stopped = self.stopped.subscribe();
        tokio::select! {
            () = tokio::time::sleep(self.delay) => {}
            _ = stopped.wait_for(|stopped| *stopped) => {
                return Ok(LoginCompletion {
                    success: false,
                    error: Some("The scripted sign-in was cancelled.".to_owned()),
                });
            }
        }
        tokio::fs::write(self.home.join(AUTH_FILE), "{}\n").await.map_err(io_error)?;
        Ok(LoginCompletion { success: true, error: None })
    }

    async fn cancel_login(&self, _login_id: &str) -> std::result::Result<(), ProviderError> {
        self.stopped.send_replace(true);
        Ok(())
    }

    async fn logout(&self) -> std::result::Result<(), ProviderError> {
        match tokio::fs::remove_file(self.home.join(AUTH_FILE)).await {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(io_error(error)),
            _ => Ok(()),
        }
    }

    async fn close(&self) {
        self.stopped.send_replace(true);
    }
}
