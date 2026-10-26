//! `limina dev`: a credential-free instance for console development and demos.
//!
//! A scripted engine ([`limina_server::dev`]) plays a deterministic research project, so the
//! console can be developed and demonstrated without provider accounts. Configuration comes from
//! the `LIMINA_DEV_*` variables (see [`DevOptions`]); with `LIMINA_DEV_SEED=1` it plays the demo
//! until Limina asks its first question and exits, leaving a ready-to-answer demo database.
//!
//! `LIMINA_DEV_IMPORT=<path>` also imports a file-based Limina project (a folder with `kb/`) the
//! first time, as a draft named after the folder (or `LIMINA_DEV_IMPORT_SLUG`), so a real project
//! can serve as this machine's development data.

use limina_core::{LiminaError, Result};
use limina_server::app::Runtime;
use limina_server::auth::AuthConfig;
use limina_server::dev::{self, DevOptions};
use limina_server::server::{ServerConfig, serve_runtime};

/// Runs the development instance on a multi-threaded runtime of its own.
pub async fn run() -> Result<()> {
    let serving = tokio::task::spawn_blocking(|| {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("limina-dev")
            .build()
            .map_err(limina_core::internal)?;
        runtime.block_on(instance())
    });
    serving.await.map_err(|error| LiminaError::generic(format!("The server stopped: {error}")))?
}

/// Imports `LIMINA_DEV_IMPORT` once; later starts find the project and leave it alone.
async fn import_local_project(runtime: &Runtime) -> Result<()> {
    use limina_domain::import::{ImportOptions, import_knowledge_base};

    let Some(path) = std::env::var_os("LIMINA_DEV_IMPORT").filter(|p| !p.is_empty()) else {
        return Ok(());
    };
    let path = std::path::PathBuf::from(path);
    let kb = if path.join("mission").join("CHALLENGE.md").is_file() {
        path.clone()
    } else {
        path.join("kb")
    };
    let folder = if path.ends_with("kb") { path.parent().unwrap_or(&path) } else { &path };
    let slug = std::env::var("LIMINA_DEV_IMPORT_SLUG")
        .ok()
        .filter(|slug| !slug.trim().is_empty())
        .unwrap_or_else(|| {
            crate::commands::slugify(&folder.file_name().unwrap_or_default().to_string_lossy())
        });
    if runtime.service().get_challenge(&slug).await.is_ok() {
        return Ok(());
    }
    let report = import_knowledge_base(
        runtime.service(),
        &kb,
        ImportOptions {
            slug,
            name: None,
            runtime_engine: None,
            owner: None,
            actor: "Dev".to_owned(),
        },
    )
    .await?;
    crate::output::error_line(&format!(
        "  Imported {} as a draft: {} hypotheses, {} experiments, {} findings.",
        report.slug, report.hypotheses, report.experiments, report.findings
    ));
    Ok(())
}

async fn instance() -> Result<()> {
    let options = DevOptions::from_env()?;
    let runtime_config = dev::prepare(&options).await?;
    let auth = AuthConfig {
        api_token: Some(options.api_token.clone()),
        admin_api_token: Some(options.admin_token.clone()),
        ..AuthConfig::default()
    };
    let mut config = ServerConfig::new(runtime_config, auth);
    "127.0.0.1".clone_into(&mut config.host);
    config.port = options.port;
    config.web_dir.clone_from(&options.web_dir);
    config.runtime.upload_limit_bytes = config.upload_limit_bytes;
    let runtime = Runtime::new(config.runtime.clone()).await?;
    dev::seed_demo(&runtime).await?;
    import_local_project(&runtime).await?;

    if !options.seed_only {
        crate::output::error_line(&options.startup_notice());
        return serve_runtime(runtime, config, crate::serve::shutdown_signal()).await;
    }
    crate::output::error_line(&format!("  Seeding the '{}' demo…", dev::DEMO_SLUG));
    let (seeded, done) = tokio::sync::oneshot::channel::<String>();
    let watched = runtime.clone();
    tokio::spawn(async move {
        let message = match dev::wait_until_seeded(&watched).await {
            Ok(outcome) => outcome.message(),
            Err(error) => format!("  Seeding failed: {error}"),
        };
        let _ = seeded.send(message);
    });
    let (report, reported) = tokio::sync::oneshot::channel::<String>();
    let shutdown = async move {
        tokio::select! {
            message = done => {
                let _ = report.send(message.unwrap_or_default());
            }
            () = crate::serve::shutdown_signal() => {}
        }
    };
    serve_runtime(runtime, config, shutdown).await?;
    if let Ok(message) = reported.await {
        crate::output::error_line(&message);
    }
    Ok(())
}
