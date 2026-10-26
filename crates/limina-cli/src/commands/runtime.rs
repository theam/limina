//! `limina runtime codex …`: the server-owned Codex login.

use std::time::Duration;

use serde_json::Value;

use super::{Context, Failure, Outcome, command_id};
use crate::cli::{CodexCommand, RuntimeCommand};
use crate::output::{self, display, field, panel, truthy};

const LOGIN_METHODS: [&str; 3] = ["chatgpt", "api-key", "access-token"];

fn render_codex_auth(value: &Value) -> String {
    let or = |key: &str, fallback: &str| match value.get(key) {
        Some(item) if truthy(item) => display(item),
        _ => fallback.to_owned(),
    };
    let mode = value.get("configured_mode").map(display).unwrap_or_else(|| "unknown".to_owned());
    panel(
        "Codex authentication",
        &format!(
            "Method  {}\nSource  {}\nAccount {}\nMode    {}",
            or("active_method", "not authenticated"),
            or("source", "none"),
            or("account_email", "not disclosed"),
            mode
        ),
    )
}

pub async fn runtime(context: &Context, command: RuntimeCommand) -> Outcome {
    let RuntimeCommand::Codex(command) = command;
    match command {
        CodexCommand::Status => {
            let result = context.admin().codex_auth_status().await?;
            context.out.emit(&result, render_codex_auth);
            Ok(())
        }
        CodexCommand::Logout => {
            let result = context.admin().codex_logout().await?;
            context.out.emit(&result, render_codex_auth);
            Ok(())
        }
        CodexCommand::Login(args) => login(context, &args.method).await,
    }
}

/// Starts a login. Device logins (ChatGPT) are polled until approved, failed or cancelled with
/// Ctrl-C; machine credentials (API key, access token) are read on the server.
async fn login(context: &Context, method: &str) -> Outcome {
    let normalized = method.trim().to_lowercase();
    if !LOGIN_METHODS.contains(&normalized.as_str()) {
        return Err(Failure::Usage {
            command: &["runtime", "codex", "login"],
            message: "Invalid value: method must be chatgpt, api-key, or access-token".to_owned(),
        });
    }
    let client = context.admin();
    let mut result = client.codex_login(&normalized, &command_id()).await?;
    let Some(login_id) = result.get("login_id").map(display) else {
        context.out.emit(&result, render_codex_auth);
        return Ok(());
    };
    if context.out.as_json {
        context.out.emit(&result, |_| String::new());
        return Ok(());
    }
    output::line(&panel(
        "Sign in with ChatGPT",
        &format!(
            "Open {}\nEnter code {}\n\nLimina will continue as soon as the login is approved.",
            field(&result, "verification_url"),
            field(&result, "user_code")
        ),
    ));
    let poll = async {
        while field(&result, "status") == "PENDING" {
            tokio::time::sleep(Duration::from_secs(1)).await;
            result = client.codex_login_attempt(&login_id).await?;
        }
        Ok::<_, Failure>(())
    };
    let interrupted = tokio::select! {
        polled = poll => { polled?; false }
        _ = tokio::signal::ctrl_c() => true,
    };
    if interrupted {
        result = client.cancel_codex_login(&login_id).await?;
    }
    output::line(&field(&result, "status").to_lowercase());
    if let Some(error) = result.get("error").filter(|value| truthy(value)) {
        output::error_line(&display(error));
    }
    Ok(())
}
