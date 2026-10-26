//! The `limina` command: project operations against a Limina instance, the hidden `_agent`
//! research protocol for managed turns, database administration and `serve`.
//!
//! Commands, options, environment variables, JSON output and exit codes are compatible with the
//! original Python CLI.

pub mod cli;
pub mod client;
pub mod commands;
pub mod dev;
pub mod output;
pub mod pyjson;
pub mod serve;

use std::ffi::OsString;

use clap::{CommandFactory, FromArgMatches};

use cli::{Cli, Command};
use commands::Failure;
use output::Output;

/// Runs the command line with `args` (including the program name) and returns the exit code.
pub fn run<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::command().try_get_matches_from(args) {
        Ok(matches) => match Cli::from_arg_matches(&matches) {
            Ok(cli) => cli,
            Err(error) => return report_clap(&error),
        },
        Err(error) => return report_clap(&error),
    };
    let no_color =
        cli.no_color || std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
    init_logging(matches!(cli.command, Command::Serve(_) | Command::Dev), no_color);
    let out = Output { as_json: cli.json };
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => return out.error(&limina_core::internal(error)),
    };
    let result = runtime.block_on(commands::dispatch(cli));
    // Do not wait for blocking stdin readers (attach, prompts) that are still parked.
    runtime.shutdown_background();
    match result {
        Ok(()) => 0,
        Err(Failure::Limina(error)) => out.error(&error),
        Err(Failure::Usage { command, message }) => usage_error(command, &message),
        Err(Failure::Aborted) => {
            output::error_line("\nAborted!");
            1
        }
        Err(Failure::Exit(code)) => code,
    }
}

fn report_clap(error: &clap::Error) -> i32 {
    let _ = error.print();
    error.exit_code()
}

/// Reports invalid input found after parsing the way clap reports its own: on standard error,
/// with the subcommand's usage, exit code 2.
fn usage_error(path: &[&str], message: &str) -> i32 {
    let mut command = Cli::command();
    command.build();
    for name in path {
        let Some(subcommand) = command.find_subcommand(name).cloned() else { break };
        command = subcommand;
    }
    let error = command.error(clap::error::ErrorKind::ValueValidation, message);
    report_clap(&error)
}

/// Logs go to standard error: `RUST_LOG` when set, `info` for `serve`, nothing otherwise.
fn init_logging(serving: bool, no_color: bool) {
    use tracing_subscriber::EnvFilter;
    let filter = match EnvFilter::try_from_default_env() {
        Ok(filter) => filter,
        Err(_) if serving => EnvFilter::new("info"),
        Err(_) => return,
    };
    // Colors only on a terminal; container logs and files get plain text.
    let ansi = !no_color && std::io::IsTerminal::is_terminal(&std::io::stderr());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(ansi)
        .with_writer(std::io::stderr)
        .try_init();
}
