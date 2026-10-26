//! The `limina` command tree: names, options, environment variables, defaults and help text of
//! the Python CLI.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::serve::ServeOptions;

/// The database the hidden `--database` option points at by default (embedded `_agent` mode and
/// `db` commands).
pub const DEFAULT_DATABASE_PATH: &str = ".limina/runtime.db";
/// The instance the public commands talk to by default.
pub const DEFAULT_URL: &str = "http://127.0.0.1:7433";

/// Operate durable Limina projects: provide missions and resources, review work, and steer the
/// managed runtime.
#[derive(Debug, Parser)]
#[command(
    name = "limina",
    version,
    arg_required_else_help = true,
    disable_help_subcommand = true,
    about = "Operate durable Limina projects: provide missions and resources, review work, and \
             steer the managed runtime."
)]
pub struct Cli {
    /// Limina instance URL.
    #[arg(long, env = "LIMINA_URL", hide_env_values = true, default_value = DEFAULT_URL, value_name = "TEXT")]
    pub url: String,

    /// Instance access token.
    #[arg(long, env = "LIMINA_API_TOKEN", hide_env_values = true, value_name = "TEXT")]
    pub token: Option<String>,

    /// Instance-administrator token for runtime configuration.
    #[arg(long, env = "LIMINA_ADMIN_API_TOKEN", hide_env_values = true, value_name = "TEXT")]
    pub admin_token: Option<String>,

    /// Your team identity.
    #[arg(long, env = "LIMINA_ACTOR", hide_env_values = true, default_value_t = default_actor(), value_name = "TEXT")]
    pub actor: String,

    #[arg(long, env = "LIMINA_DATABASE_URL", hide_env_values = true, default_value = DEFAULT_DATABASE_PATH, hide = true)]
    pub database: String,

    /// Emit machine-readable JSON.
    #[arg(long = "json")]
    pub json: bool,

    /// Disable ANSI styling.
    #[arg(long)]
    pub no_color: bool,

    #[command(subcommand)]
    pub command: Command,
}

/// `os.environ.get("USER", "unknown")`.
fn default_actor() -> String {
    std::env::var("USER").unwrap_or_else(|_| "unknown".to_owned())
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Verify connectivity, authentication, and runtime ownership.
    #[command(verbatim_doc_comment)]
    Doctor,
    /// Run a complete Limina instance, including all managed project runtimes.
    #[command(verbatim_doc_comment)]
    Serve(ServeOptions),
    /// Run a credential-free development instance with a scripted engine and a demo project.
    #[command(hide = true)]
    Dev,
    /// Start autonomous work on a created or stopped project.
    #[command(verbatim_doc_comment)]
    Start(ProjectArg),
    /// Interrupt active work safely and leave the project resumable.
    #[command(verbatim_doc_comment)]
    Pause(ProjectArg),
    /// Resume a paused, waiting, stopped, or failed project.
    #[command(verbatim_doc_comment)]
    Resume(ProjectArg),
    /// Stop execution while preserving all knowledge and history.
    #[command(verbatim_doc_comment)]
    Stop(ProjectArg),
    /// Show mission progress without exposing runtime machinery.
    #[command(verbatim_doc_comment)]
    Status(ProjectArg),
    /// Review accepted work and knowledge; optionally open one artifact.
    #[command(verbatim_doc_comment)]
    Review(ReviewArgs),
    /// Deliver durable guidance, live when a turn is active and queued otherwise.
    #[command(verbatim_doc_comment)]
    Steer(SteerArgs),
    /// Follow the durable project activity stream without entering interactive mode.
    #[command(verbatim_doc_comment)]
    Watch(WatchArgs),
    /// Enter the live project: observe Limina working and steer the active turn.
    #[command(verbatim_doc_comment)]
    Attach(ProjectArg),
    /// Export a deterministic, validator-compatible Markdown knowledge base.
    #[command(verbatim_doc_comment)]
    Export(ExportArgs),
    /// Create and manage Limina projects.
    #[command(verbatim_doc_comment)]
    #[command(subcommand, arg_required_else_help = true)]
    Project(ProjectCommand),
    /// Manage resources Limina may consume.
    #[command(verbatim_doc_comment)]
    #[command(subcommand, arg_required_else_help = true)]
    Resource(ResourceCommand),
    /// Administer managed runtime engines.
    #[command(verbatim_doc_comment)]
    #[command(subcommand, arg_required_else_help = true)]
    Runtime(RuntimeCommand),
    #[command(subcommand, hide = true, arg_required_else_help = true)]
    Db(DbCommand),
    #[command(name = "_agent", subcommand, hide = true, arg_required_else_help = true)]
    Agent(AgentCommand),
    /// Exit 0 when the URL answers 2xx (the container health check).
    #[command(verbatim_doc_comment)]
    #[command(name = "_healthcheck", hide = true)]
    Healthcheck(HealthcheckArgs),
}

#[derive(Debug, Args)]
pub struct ProjectArg {
    pub project: String,
}

#[derive(Debug, Args)]
pub struct ReviewArgs {
    pub project: String,
    /// Open one H/E/F artifact.
    #[arg(verbatim_doc_comment)]
    #[arg(short = 'a', long, value_name = "TEXT")]
    pub artifact: Option<String>,
}

#[derive(Debug, Args)]
pub struct SteerArgs {
    pub project: String,
    /// Feedback or strategic direction.
    #[arg(verbatim_doc_comment)]
    pub message: String,
    /// STEER, ANSWER, APPROVAL, COMMENT, BLOCKER, or INTERRUPT.
    #[arg(verbatim_doc_comment)]
    #[arg(long, default_value = "STEER", value_name = "TEXT")]
    pub kind: String,
}

#[derive(Debug, Args)]
pub struct WatchArgs {
    pub project: String,
    /// Resume after this event cursor.
    #[arg(verbatim_doc_comment)]
    #[arg(long, default_value_t = 0, value_name = "INTEGER", allow_negative_numbers = true)]
    pub after: i64,
    /// Keep following new activity (the default).
    #[arg(verbatim_doc_comment)]
    #[arg(long, overrides_with = "no_follow")]
    pub follow: bool,
    /// Print the current activity and exit.
    #[arg(verbatim_doc_comment)]
    #[arg(long, overrides_with = "follow")]
    pub no_follow: bool,
    #[arg(long, default_value = "1.0", value_name = "FLOAT")]
    pub interval: f64,
}

impl WatchArgs {
    pub fn following(&self) -> bool {
        !self.no_follow
    }
}

#[derive(Debug, Args)]
pub struct ExportArgs {
    pub project: String,
    /// Empty target directory.
    #[arg(verbatim_doc_comment)]
    #[arg(default_value = "kb-export", value_name = "TARGET")]
    pub target: PathBuf,
}

#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// Create a project from a mission; execution begins only when you start it.
    #[command(verbatim_doc_comment)]
    Create(CreateProjectArgs),
    /// List the projects managed by this Limina instance.
    #[command(verbatim_doc_comment)]
    List(ListProjectsArgs),
    /// Show one project's mission and current runtime state.
    #[command(verbatim_doc_comment)]
    Show(ProjectArg),
    /// Archive a project that is no longer running.
    #[command(verbatim_doc_comment)]
    Archive(ProjectArg),
    /// Import a file-based Limina project (its kb/ folder) as a draft project.
    #[command(verbatim_doc_comment)]
    Import(ImportProjectArgs),
    /// Give someone access to a project, straight in the database (--database).
    /// With --subject they become a member at once; otherwise the access applies
    /// the first time they sign in with this verified email.
    #[command(verbatim_doc_comment)]
    Share(ShareProjectArgs),
}

#[derive(Debug, Args)]
pub struct ShareProjectArgs {
    pub project: String,
    /// The person's email.
    #[arg(verbatim_doc_comment)]
    #[arg(long, value_name = "EMAIL")]
    pub email: String,
    /// Their identity-provider user ID (for example a WorkOS user_… ID).
    #[arg(verbatim_doc_comment)]
    #[arg(long, value_name = "SUBJECT")]
    pub subject: Option<String>,
    /// Their display name; defaults to the email.
    #[arg(verbatim_doc_comment)]
    #[arg(long, value_name = "TEXT", requires = "subject")]
    pub name: Option<String>,
    /// OWNER, EDITOR, or VIEWER.
    #[arg(verbatim_doc_comment)]
    #[arg(long, value_enum, ignore_case = true, default_value_t = MemberRole::Owner)]
    pub role: MemberRole,
}

/// A project role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum MemberRole {
    #[value(name = "OWNER")]
    Owner,
    #[value(name = "EDITOR")]
    Editor,
    #[value(name = "VIEWER")]
    Viewer,
}

impl MemberRole {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Owner => "OWNER",
            Self::Editor => "EDITOR",
            Self::Viewer => "VIEWER",
        }
    }
}

#[derive(Debug, Args)]
pub struct ImportProjectArgs {
    /// The project folder, or its kb/ folder.
    #[arg(verbatim_doc_comment, value_name = "PATH")]
    pub path: PathBuf,
    /// Project slug; defaults to the project folder's name.
    #[arg(verbatim_doc_comment)]
    #[arg(long, value_name = "TEXT")]
    pub slug: Option<String>,
    /// Display name; defaults to the mission title.
    #[arg(verbatim_doc_comment)]
    #[arg(long, value_name = "TEXT")]
    pub name: Option<String>,
    /// Managed execution engine: codex or claude-code.
    #[arg(verbatim_doc_comment)]
    #[arg(long, value_enum, default_value_t = RuntimeEngine::Codex)]
    pub runtime: RuntimeEngine,
    /// Identity-provider user ID of the project's owner (for example a WorkOS user_… ID).
    #[arg(verbatim_doc_comment)]
    #[arg(long, value_name = "SUBJECT")]
    pub owner: Option<String>,
    /// The owner's display name.
    #[arg(verbatim_doc_comment)]
    #[arg(long, value_name = "TEXT", requires = "owner")]
    pub owner_name: Option<String>,
    /// The owner's email.
    #[arg(verbatim_doc_comment)]
    #[arg(long, value_name = "EMAIL", requires = "owner")]
    pub owner_email: Option<String>,
}

/// A managed execution engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RuntimeEngine {
    #[value(name = "codex")]
    Codex,
    #[value(name = "claude-code")]
    ClaudeCode,
}

impl RuntimeEngine {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude-code",
        }
    }
}

#[derive(Debug, Args)]
pub struct CreateProjectArgs {
    /// Stable lowercase project slug.
    #[arg(verbatim_doc_comment)]
    pub slug: String,
    /// Mission Limina must advance.
    #[arg(verbatim_doc_comment)]
    #[arg(short = 'm', long, value_name = "TEXT", allow_hyphen_values = true)]
    pub mission: Option<String>,
    /// Observable completion criteria.
    #[arg(verbatim_doc_comment)]
    #[arg(short = 's', long, value_name = "TEXT", allow_hyphen_values = true)]
    pub success: Option<String>,
    /// Human-readable name.
    #[arg(verbatim_doc_comment)]
    #[arg(long, value_name = "TEXT")]
    pub name: Option<String>,
    /// Known context, constraints, and prior work.
    #[arg(verbatim_doc_comment)]
    #[arg(long, default_value = "", hide_default_value = true, value_name = "TEXT")]
    #[arg(allow_hyphen_values = true)]
    pub context: String,
    /// Managed execution engine: codex or claude-code.
    #[arg(verbatim_doc_comment)]
    #[arg(long, value_enum, default_value_t = RuntimeEngine::Codex)]
    pub runtime: RuntimeEngine,
}

#[derive(Debug, Args)]
pub struct ListProjectsArgs {
    /// Include archived projects.
    #[arg(verbatim_doc_comment)]
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Subcommand)]
pub enum ResourceCommand {
    /// Set or replace a visible project variable.
    #[command(verbatim_doc_comment)]
    Variable(VariableArgs),
    /// Set or rotate a write-only encrypted project secret.
    #[command(verbatim_doc_comment)]
    Secret(SecretArgs),
    /// List resources available to a project.
    #[command(verbatim_doc_comment)]
    List(ProjectArg),
    /// Remove a project variable or secret and wipe its stored value.
    #[command(verbatim_doc_comment)]
    Remove(ResourceNameArgs),
}

#[derive(Debug, Args)]
pub struct VariableArgs {
    pub project: String,
    pub name: String,
    /// Visible value to provide to the project.
    #[arg(verbatim_doc_comment)]
    #[arg(allow_hyphen_values = true)]
    pub value: String,
}

#[derive(Debug, Args)]
pub struct SecretArgs {
    pub project: String,
    pub name: String,
    /// Read the value from this local environment variable.
    #[arg(verbatim_doc_comment)]
    #[arg(long, value_name = "TEXT")]
    pub from_env: Option<String>,
    /// Read the value from standard input.
    #[arg(verbatim_doc_comment)]
    #[arg(long)]
    pub from_stdin: bool,
}

#[derive(Debug, Args)]
pub struct ResourceNameArgs {
    pub project: String,
    pub name: String,
}

#[derive(Debug, Subcommand)]
pub enum RuntimeCommand {
    /// Administer Codex authentication.
    #[command(verbatim_doc_comment)]
    #[command(subcommand, arg_required_else_help = true)]
    Codex(CodexCommand),
}

#[derive(Debug, Subcommand)]
pub enum CodexCommand {
    /// Show the server-owned Codex login state.
    #[command(verbatim_doc_comment)]
    Status,
    /// Authenticate Codex for every managed project on this runtime node.
    #[command(verbatim_doc_comment)]
    Login(CodexLoginArgs),
    /// Remove the server-owned Codex login.
    #[command(verbatim_doc_comment)]
    Logout,
}

#[derive(Debug, Args)]
pub struct CodexLoginArgs {
    /// chatgpt, api-key, or access-token; machine credentials stay on the server.
    #[arg(verbatim_doc_comment)]
    #[arg(long, default_value = "chatgpt", value_name = "TEXT")]
    pub method: String,
}

#[derive(Debug, Subcommand)]
pub enum DbCommand {
    Upgrade,
    Current,
}

#[derive(Debug, Args)]
pub struct HealthcheckArgs {
    #[arg(long, default_value = "http://127.0.0.1:7433/readyz", value_name = "URL")]
    pub url: String,
}

// The hidden research protocol managed agents call during a turn. Arguments, defaults and JSON
// output are a contract with the agent prompts.
#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    Status,
    List(AgentListArgs),
    Show(AgentShowArgs),
    #[command(subcommand, arg_required_else_help = true)]
    Hypothesis(AgentHypothesisCommand),
    #[command(subcommand, arg_required_else_help = true)]
    Experiment(AgentExperimentCommand),
    #[command(subcommand, arg_required_else_help = true)]
    Finding(AgentFindingCommand),
}

#[derive(Debug, Args)]
pub struct AgentListArgs {
    #[arg(long, value_name = "TEXT")]
    pub kind: Option<String>,
}

#[derive(Debug, Args)]
pub struct AgentShowArgs {
    pub artifact_id: String,
}

#[derive(Debug, Subcommand)]
pub enum AgentHypothesisCommand {
    Add(HypothesisAddArgs),
    Decide(HypothesisDecideArgs),
}

#[derive(Debug, Args)]
pub struct HypothesisAddArgs {
    pub title: String,
    #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
    pub statement: String,
    #[arg(long, default_value = "", hide_default_value = true, value_name = "TEXT")]
    #[arg(allow_hyphen_values = true)]
    pub mechanism: String,
    #[arg(long, default_value = "", hide_default_value = true, value_name = "TEXT")]
    #[arg(allow_hyphen_values = true)]
    pub generalization: String,
    #[arg(long, default_value = "", hide_default_value = true, value_name = "TEXT")]
    #[arg(allow_hyphen_values = true)]
    pub shortcut_risks: String,
    #[arg(long, default_value = "", hide_default_value = true, value_name = "TEXT")]
    #[arg(allow_hyphen_values = true)]
    pub test_plan: String,
}

#[derive(Debug, Args)]
pub struct HypothesisDecideArgs {
    pub artifact_id: String,
    #[arg(long, value_name = "TEXT")]
    pub status: String,
    #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
    pub conclusion: String,
    #[arg(long, value_name = "INTEGER", allow_negative_numbers = true)]
    pub expect: i64,
}

#[derive(Debug, Subcommand)]
pub enum AgentExperimentCommand {
    Add(ExperimentAddArgs),
    Claim(ExperimentClaimArgs),
    Observe(ExperimentObserveArgs),
    Complete(ExperimentCompleteArgs),
}

#[derive(Debug, Args)]
pub struct ExperimentAddArgs {
    pub hypothesis_id: String,
    pub title: String,
    #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
    pub objective: String,
    #[arg(long, default_value = "", hide_default_value = true, value_name = "TEXT")]
    #[arg(allow_hyphen_values = true)]
    pub procedure: String,
    #[arg(long, default_value = "", hide_default_value = true, value_name = "TEXT")]
    #[arg(allow_hyphen_values = true)]
    pub success: String,
    #[arg(long, default_value = "", hide_default_value = true, value_name = "TEXT")]
    #[arg(allow_hyphen_values = true)]
    pub guardrails: String,
}

#[derive(Debug, Args)]
pub struct ExperimentClaimArgs {
    pub artifact_id: String,
    #[arg(long, default_value_t = 1800, value_name = "INTEGER", allow_negative_numbers = true)]
    pub ttl: i64,
}

#[derive(Debug, Args)]
pub struct ExperimentObserveArgs {
    pub artifact_id: String,
    pub observation: String,
    #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
    pub evidence: Option<String>,
}

#[derive(Debug, Args)]
pub struct ExperimentCompleteArgs {
    pub artifact_id: String,
    #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
    pub results: String,
    #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
    pub analysis: String,
    #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
    pub decision: String,
    #[arg(long, value_name = "INTEGER", allow_negative_numbers = true)]
    pub expect: i64,
}

#[derive(Debug, Subcommand)]
pub enum AgentFindingCommand {
    Publish(FindingPublishArgs),
}

#[derive(Debug, Args)]
pub struct FindingPublishArgs {
    pub experiment_id: String,
    pub title: String,
    #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
    pub finding: String,
    #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
    pub evidence: String,
    #[arg(long, default_value = "", hide_default_value = true, value_name = "TEXT")]
    #[arg(allow_hyphen_values = true)]
    pub improvement: String,
    #[arg(long, default_value = "", hide_default_value = true, value_name = "TEXT")]
    #[arg(allow_hyphen_values = true)]
    pub debt: String,
    #[arg(long = "next", default_value = "", hide_default_value = true, value_name = "TEXT")]
    #[arg(allow_hyphen_values = true)]
    pub next_move: String,
    #[arg(long, default_value = "HIGH", value_name = "TEXT")]
    pub impact: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_command_tree_is_well_formed() {
        Cli::command().debug_assert();
    }
}
