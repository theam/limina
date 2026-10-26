//! The development runtime's configuration and its demo project.

use std::sync::Arc;
use std::time::Duration;

use limina_core::{ErrorKind, Principal, Result, internal, new_uuid};
use limina_engines::EngineAvailability;

use super::agent::{SCRIPTED_MODEL, scripted_factory};
use super::codex::ScriptedCodexConnector;
use super::options::DevOptions;
use crate::app::{EngineStatusOverride, NewProject, Runtime, RuntimeConfig};

/// The seeded demo project.
pub const DEMO_SLUG: &str = "retrieval-reliability-demo";
/// Who creates and starts the demo (a trusted local operator).
pub const DEMO_ACTOR: &str = "Dev";
/// What the engines report with shared credentials.
pub const SCRIPTED_ENGINE_DETAIL: &str = "Scripted development engine (no provider calls).";
/// What the engines report with `LIMINA_DEV_SHARED_CREDENTIALS=0`.
pub const NO_SHARED_CREDENTIALS_DETAIL: &str =
    "No shared credentials on this dev runtime; connect an account to the project.";
/// How often [`wait_until_seeded`] looks at the demo.
const SEED_POLL: Duration = Duration::from_millis(500);

/// The runtime configuration of a development runtime: the scripted engine for every project,
/// the scripted Codex sign-in, engine readiness that needs no provider, and the state under
/// `options.data`. Other settings are read from the environment as
/// [`RuntimeConfig::from_env`] does. Creates the data directory.
pub async fn prepare(options: &DevOptions) -> Result<RuntimeConfig> {
    tokio::fs::create_dir_all(&options.data).await.map_err(internal)?;
    let mut config = RuntimeConfig::from_env()?;
    config.database_url = options.database_url.clone();
    config.workspace_root = options.data.join("workspaces");
    config.secret_key_path = options.data.join("secret.key");
    config.internal_url = options.internal_url();
    config.agent_factory = Some(scripted_factory(options.pace));
    config.codex_connector =
        Some(Arc::new(ScriptedCodexConnector::with_delay(options.sign_in_delay)));
    config.engine_status = Some(scripted_engine_status(options.shared_credentials));
    Ok(config)
}

/// Engine readiness for the scripted engine: installed, with shared credentials unless
/// `shared_credentials` is off (projects must then connect their own accounts).
pub fn scripted_engine_status(shared_credentials: bool) -> EngineStatusOverride {
    Arc::new(move |engine| EngineAvailability {
        engine: engine.id().to_owned(),
        label: engine.label().to_owned(),
        installed: true,
        credentials: shared_credentials,
        detail: if shared_credentials {
            SCRIPTED_ENGINE_DETAIL
        } else {
            NO_SHARED_CREDENTIALS_DETAIL
        }
        .to_owned(),
        model: Some(SCRIPTED_MODEL.to_owned()),
    })
}

/// Creates and starts the demo project unless it already exists, and starts its loop.
pub async fn seed_demo(runtime: &Runtime) -> Result<()> {
    match runtime.service().get_challenge(DEMO_SLUG).await {
        Ok(_) => return Ok(()),
        Err(error) if error.kind == ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let project = NewProject {
        model: None,
        reasoning_effort: None,
        slug: DEMO_SLUG.to_owned(),
        name: "Retrieval reliability (demo)".to_owned(),
        mission: "Reduce hallucinated citations in the support assistant’s retrieval-augmented \
                  answers without hurting its answer rate."
            .to_owned(),
        success_criteria: "Unsupported citations below 8% on the held-out test split with at \
                           most a 2-point drop in answer rate."
            .to_owned(),
        context: "Help-center corpus of ~14k articles. Evaluation set: 412 dev / 390 test \
                  questions with annotated supporting passages. The generator and prompts are \
                  frozen for this study.\n\n\
                  Seeded by Limina's scripted demo engine: the history and numbers are \
                  illustrative, not measured."
            .to_owned(),
        runtime: Some("codex".to_owned()),
    };
    let principal = Principal::local(DEMO_ACTOR, false);
    runtime.operations().create_project(project, &new_uuid(), &principal).await?;
    runtime.service().change_project_state(DEMO_SLUG, "start", DEMO_ACTOR, &new_uuid()).await?;
    runtime.supervisor().ensure_running(DEMO_SLUG).await
}

/// Where the demo stands once seeding is over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedOutcome {
    /// The demo waits on a human decision: the coordinator's blocker.
    Waiting { question: String },
    /// The demo failed: the coordinator's blocker.
    Failed { blocker: String },
}

impl SeedOutcome {
    /// What the script printed when it stopped.
    pub fn message(&self) -> String {
        match self {
            Self::Waiting { .. } => format!("  Seeded '{DEMO_SLUG}': waiting on a human decision."),
            Self::Failed { blocker } => format!("  Seeding failed: {blocker}"),
        }
    }
}

/// Resolves once the demo waits on its first human decision (`WAITING` with a blocker), or has
/// failed. Seed mode (`LIMINA_DEV_SEED`) stops serving then.
pub async fn wait_until_seeded(runtime: &Runtime) -> Result<SeedOutcome> {
    loop {
        tokio::time::sleep(SEED_POLL).await;
        let Some(coordinator) = runtime.service().get_challenge(DEMO_SLUG).await?.coordinator
        else {
            continue;
        };
        let blocker = coordinator.blocker.trim();
        if coordinator.status == "WAITING" && !matches!(blocker, "" | "None") {
            return Ok(SeedOutcome::Waiting { question: coordinator.blocker });
        }
        if coordinator.status == "FAILED" {
            return Ok(SeedOutcome::Failed { blocker: coordinator.blocker });
        }
    }
}
