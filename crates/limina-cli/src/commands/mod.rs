//! Command handlers. Each returns `Ok(())` after printing its result, or a [`Failure`] the entry
//! point reports with the right exit code.

mod agent;
mod attach;
mod database;
mod health;
mod project;
mod runtime;

use std::sync::OnceLock;

use limina_core::LiminaError;

use crate::cli::{Cli, Command};
use crate::client::HttpRuntimeClient;
use crate::output::Output;

/// Why a command did not complete.
#[derive(Debug)]
pub enum Failure {
    /// A domain or transport error, reported like the Python CLI's `_invoke`.
    Limina(LiminaError),
    /// Invalid input found after parsing (`typer.BadParameter`): usage error, exit code 2.
    Usage { command: &'static [&'static str], message: String },
    /// The user aborted a prompt (end of input or Ctrl-C): exit code 1.
    Aborted,
    /// A command that decides its own exit code (`_healthcheck`).
    Exit(i32),
}

impl From<LiminaError> for Failure {
    fn from(error: LiminaError) -> Self {
        Self::Limina(error)
    }
}

pub type Outcome = Result<(), Failure>;

/// What every command shares: the global options and the lazily created public clients.
#[derive(Debug)]
pub struct Context {
    pub url: String,
    pub token: Option<String>,
    pub admin_token: Option<String>,
    pub actor: String,
    pub database: String,
    pub out: Output,
    public: OnceLock<HttpRuntimeClient>,
    admin: OnceLock<HttpRuntimeClient>,
}

impl Context {
    pub fn new(cli: &Cli) -> Self {
        Self {
            url: cli.url.clone(),
            token: cli.token.clone(),
            admin_token: cli.admin_token.clone(),
            actor: cli.actor.clone(),
            database: cli.database.clone(),
            out: Output { as_json: cli.json },
            public: OnceLock::new(),
            admin: OnceLock::new(),
        }
    }

    /// The client for project commands, authenticated with `--token`.
    pub fn public(&self) -> &HttpRuntimeClient {
        self.public.get_or_init(|| HttpRuntimeClient::new(&self.url, self.token.as_deref(), None))
    }

    /// The client for runtime administration, authenticated with `--admin-token` (or `--token`).
    pub fn admin(&self) -> &HttpRuntimeClient {
        self.admin.get_or_init(|| {
            let token = self.admin_token.as_deref().filter(|t| !t.is_empty());
            HttpRuntimeClient::new(&self.url, token.or(self.token.as_deref()), None)
        })
    }
}

/// A new command ID (`Idempotency-Key`).
pub fn command_id() -> String {
    limina_core::new_uuid()
}

/// `sqlite:///<path>` for a bare path (`~` expanded); URLs pass through.
pub fn database_url(value: &str) -> String {
    if value.contains("://") {
        return value.to_owned();
    }
    let expanded = match value.strip_prefix("~") {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => match std::env::var("HOME") {
            Ok(home) if !home.is_empty() => format!("{}{rest}", home.trim_end_matches('/')),
            _ => value.to_owned(),
        },
        _ => value.to_owned(),
    };
    format!("sqlite:///{expanded}")
}

pub use project::slugify;

pub async fn dispatch(cli: Cli) -> Outcome {
    let context = Context::new(&cli);
    match cli.command {
        Command::Doctor => project::doctor(&context).await,
        Command::Serve(options) => {
            if let Err(message) = options.validate() {
                return Err(Failure::Usage { command: &["serve"], message });
            }
            crate::serve::run(options).await.map_err(Failure::from)
        }
        Command::Dev => crate::dev::run().await.map_err(Failure::from),
        Command::Start(args) => project::lifecycle(&context, &args.project, "start").await,
        Command::Pause(args) => project::lifecycle(&context, &args.project, "pause").await,
        Command::Resume(args) => project::lifecycle(&context, &args.project, "resume").await,
        Command::Stop(args) => project::lifecycle(&context, &args.project, "stop").await,
        Command::Status(args) => project::status(&context, &args.project).await,
        Command::Review(args) => project::review(&context, &args).await,
        Command::Steer(args) => project::steer(&context, &args).await,
        Command::Watch(args) => project::watch(&context, &args).await,
        Command::Attach(args) => attach::attach(&context, &args.project).await,
        Command::Export(args) => project::export(&context, &args).await,
        Command::Project(command) => project::project(&context, command).await,
        Command::Resource(command) => project::resource(&context, command).await,
        Command::Runtime(command) => runtime::runtime(&context, command).await,
        Command::Db(command) => database::database(&context, &command).await,
        Command::Agent(command) => agent::agent(&context, command).await,
        Command::Healthcheck(args) => health::healthcheck(&args.url).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_paths_become_sqlite_urls() {
        assert_eq!(database_url(".limina/runtime.db"), "sqlite:///.limina/runtime.db");
        assert_eq!(
            database_url("/var/lib/limina/runtime.db"),
            "sqlite:////var/lib/limina/runtime.db"
        );
        assert_eq!(database_url("postgresql+psycopg://u@h/db"), "postgresql+psycopg://u@h/db");
        assert_eq!(database_url("sqlite:///x.db"), "sqlite:///x.db");
    }
}
