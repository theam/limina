//! `DevOptions`: the development runtime's settings, from `LIMINA_DEV_*`.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use limina_core::{LiminaError, Result};
use limina_domain::environment::process_environment;

use super::setup::DEMO_SLUG;

/// The time unit between streamed events (`LIMINA_DEV_PACE`).
pub const DEFAULT_PACE: Duration = Duration::from_millis(1200);
/// The state directory (`LIMINA_DEV_DATA`), relative to the working directory.
pub const DEFAULT_DATA: &str = ".limina/ui-dev";
/// Where the runtime listens (`LIMINA_DEV_PORT`).
pub const DEFAULT_PORT: u16 = 7433;
/// The console's shared token (`LIMINA_API_TOKEN`).
pub const DEFAULT_API_TOKEN: &str = "dev";
/// The console's administrator token (`LIMINA_ADMIN_API_TOKEN`).
pub const DEFAULT_ADMIN_TOKEN: &str = "dev-admin";
/// How long the scripted Codex sign-in takes to confirm itself.
pub const DEFAULT_SIGN_IN_DELAY: Duration = Duration::from_secs(6);
/// The console build served when it exists, relative to the working directory.
const WEB_DIR: &str = "web/dist";
/// The Vite development server, used in the startup notice when there is no build.
const VITE_URL: &str = "http://127.0.0.1:5173";
/// A pace above this is a typo, not a demo.
const MAX_PACE_SECONDS: f64 = 3600.0;

/// How the development runtime runs. [`DevOptions::from_env`] reads the environment; every field
/// can be changed afterwards (tests use a tiny pace and their own directories).
#[derive(Clone, PartialEq, Eq)]
pub struct DevOptions {
    /// `LIMINA_DEV_PACE` (seconds, default 1.2): every scripted step waits a multiple of it.
    pub pace: Duration,
    /// `LIMINA_DEV_DATA`: holds `runtime.db`, `workspaces/`, the Codex homes and `secret.key`.
    pub data: PathBuf,
    /// `LIMINA_DEV_DATABASE_URL` (default `sqlite:///<data>/runtime.db`).
    pub database_url: String,
    /// `LIMINA_DEV_SEED` (`1`, `true` or `yes`): play the demo until Limina asks its first
    /// question, then exit.
    pub seed_only: bool,
    /// `LIMINA_DEV_PORT`: where the runtime listens; the internal agent API URL follows it.
    pub port: u16,
    /// `LIMINA_DEV_SHARED_CREDENTIALS` (default on; `0` or `false` turns it off): whether the
    /// runtime reports shared engine credentials, or every project must connect its own account.
    pub shared_credentials: bool,
    /// `LIMINA_API_TOKEN` (default `dev`): the shared token for people and automation.
    pub api_token: String,
    /// `LIMINA_ADMIN_API_TOKEN` (default `dev-admin`): the shared administrator token.
    pub admin_token: String,
    /// `web/dist` when it holds a console build (`index.html`), otherwise `None` (use Vite).
    pub web_dir: Option<PathBuf>,
    /// How long the scripted Codex device sign-in takes to confirm itself (6 s).
    pub sign_in_delay: Duration,
}

impl fmt::Debug for DevOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DevOptions")
            .field("pace", &self.pace)
            .field("data", &self.data)
            .field("database_url", &self.database_url)
            .field("seed_only", &self.seed_only)
            .field("port", &self.port)
            .field("shared_credentials", &self.shared_credentials)
            .field("web_dir", &self.web_dir)
            .field("sign_in_delay", &self.sign_in_delay)
            .finish_non_exhaustive()
    }
}

impl DevOptions {
    /// The options from the process environment.
    pub fn from_env() -> Result<Self> {
        Self::from_map(&process_environment())
    }

    /// The options from an explicit environment map. Empty values count as unset.
    pub fn from_map(env: &BTreeMap<String, String>) -> Result<Self> {
        let value = |name: &str| env.get(name).map(|value| value.trim()).filter(|v| !v.is_empty());
        let pace = match value("LIMINA_DEV_PACE") {
            Some(text) => parse_pace(text)?,
            None => DEFAULT_PACE,
        };
        let data = PathBuf::from(value("LIMINA_DEV_DATA").unwrap_or(DEFAULT_DATA));
        let database_url = value("LIMINA_DEV_DATABASE_URL")
            .map_or_else(|| sqlite_url(&data.join("runtime.db")), str::to_owned);
        let port = match value("LIMINA_DEV_PORT") {
            Some(text) => text.parse::<u16>().map_err(|_| {
                LiminaError::invariant("LIMINA_DEV_PORT must be a port number.").with("value", text)
            })?,
            None => DEFAULT_PORT,
        };
        let seed_only = value("LIMINA_DEV_SEED")
            .is_some_and(|flag| matches!(flag.to_lowercase().as_str(), "1" | "true" | "yes"));
        let shared_credentials = value("LIMINA_DEV_SHARED_CREDENTIALS")
            .is_none_or(|flag| !matches!(flag.to_lowercase().as_str(), "0" | "false"));
        let web_dir = PathBuf::from(WEB_DIR);
        Ok(Self {
            pace,
            data,
            database_url,
            seed_only,
            port,
            shared_credentials,
            api_token: value("LIMINA_API_TOKEN").unwrap_or(DEFAULT_API_TOKEN).to_owned(),
            admin_token: value("LIMINA_ADMIN_API_TOKEN").unwrap_or(DEFAULT_ADMIN_TOKEN).to_owned(),
            web_dir: web_dir.join("index.html").is_file().then_some(web_dir),
            sign_in_delay: DEFAULT_SIGN_IN_DELAY,
        })
    }

    /// The internal agent API as providers reach it: `http://127.0.0.1:<port>`.
    pub fn internal_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The console, signed in with the administrator token: the served build when there is one,
    /// otherwise the Vite development server.
    pub fn console_url(&self) -> String {
        let base = match self.web_dir {
            Some(_) => self.internal_url(),
            None => VITE_URL.to_owned(),
        };
        format!("{base}/#token={}", self.admin_token)
    }

    /// What the script printed on startup.
    pub fn startup_notice(&self) -> String {
        if self.seed_only {
            format!("  Seeding the '{DEMO_SLUG}' demo…")
        } else {
            format!("\n  Limina dev runtime  →  {}\n", self.console_url())
        }
    }
}

fn parse_pace(text: &str) -> Result<Duration> {
    let invalid = || {
        LiminaError::invariant(format!(
            "LIMINA_DEV_PACE must be a number of seconds between 0 and {MAX_PACE_SECONDS}."
        ))
        .with("value", text)
    };
    let seconds = text.parse::<f64>().map_err(|_| invalid())?;
    if !(0.0..=MAX_PACE_SECONDS).contains(&seconds) {
        return Err(invalid());
    }
    Duration::try_from_secs_f64(seconds).map_err(|_| invalid())
}

/// `sqlite:///<path>` (a relative path stays relative, like the Python script's URL).
fn sqlite_url(path: &Path) -> String {
    format!("sqlite:///{}", path.display())
}
