//! The public project commands: projects, lifecycle, status, review, steering, activity,
//! resources, export and the connectivity check.

use std::io::{BufRead, Read};
use std::time::Duration;

use serde_json::{Value, json};

use super::{Context, Failure, Outcome, command_id};
use crate::cli::{
    CreateProjectArgs, ExportArgs, ImportProjectArgs, ProjectCommand, ResourceCommand, ReviewArgs,
    SecretArgs, ShareProjectArgs, SteerArgs, WatchArgs,
};
use crate::client::{Command, write_snapshot};
use crate::output::{self, display, engine_label, field, panel, table, title_case, truthy};

/// The project card (`_render_project`).
pub fn render_project(project: &Value) -> String {
    let body = format!(
        "{}\n{}\n\nEngine     {}\nStatus     {}\nObjective  {}\nNext       {}\nBlocker    {}",
        field(project, "name"),
        field(project, "mission"),
        project.get("runtime").map(engine_label).unwrap_or_default(),
        field(project, "status"),
        field(project, "current_objective"),
        field(project, "next_step"),
        field(project, "blocker"),
    );
    panel(&field(project, "slug"), &body)
}

/// One activity line (`_render_activity`).
pub fn render_activity(event: &Value) -> String {
    let empty = Value::Null;
    let detail = event.get("detail").unwrap_or(&empty);
    let summary = ["summary", "title"]
        .iter()
        .filter_map(|key| detail.get(*key))
        .find(|value| truthy(value))
        .map(display)
        .unwrap_or_default();
    let artifact = match event.get("artifact_id") {
        Some(value) if truthy(value) => format!(" {}", display(value)),
        _ => String::new(),
    };
    let created_at: String = field(event, "created_at").chars().take(19).collect();
    format!(
        "{:>5} {} {}{} by {} {}",
        field(event, "sequence"),
        created_at,
        field(event, "type"),
        artifact,
        field(event, "actor"),
        summary
    )
}

pub async fn doctor(context: &Context) -> Outcome {
    let result = context.public().health().await?;
    context.out.emit(&result, |value| {
        let engines = match value.get("runtimes") {
            Some(Value::Array(items)) => {
                items.iter().map(engine_label).collect::<Vec<_>>().join(", ")
            }
            _ => String::new(),
        };
        format!(
            "ok {} · runtime owned by {} · engines {}",
            context.url,
            field(value, "runtime_owner"),
            engines
        )
    });
    Ok(())
}

pub async fn project(context: &Context, command: ProjectCommand) -> Outcome {
    match command {
        ProjectCommand::Create(args) => create(context, args).await,
        ProjectCommand::List(args) => {
            let result = context.public().projects(args.all).await?;
            context.out.emit(&result, |values| {
                let rows: Vec<Vec<String>> = items(values)
                    .iter()
                    .map(|item| {
                        vec![
                            field(item, "slug"),
                            item.get("runtime").map(engine_label).unwrap_or_default(),
                            field(item, "status"),
                            field(item, "mission"),
                            field(item, "next_step"),
                        ]
                    })
                    .collect();
                table("Limina projects", &["Project", "Engine", "Status", "Mission", "Next"], &rows)
            });
            Ok(())
        }
        ProjectCommand::Show(args) => {
            let result = context.public().project(&args.project).await?;
            context.out.emit(&result, render_project);
            Ok(())
        }
        ProjectCommand::Archive(args) => lifecycle(context, &args.project, "archive").await,
        ProjectCommand::Import(args) => import(context, args).await,
        ProjectCommand::Share(args) => share(context, args).await,
    }
}

/// Grants access straight in the database (`--database`), like `project import`: how operators
/// give people projects that nobody can open yet, for example ones a local or demo user created
/// before the instance moved to a sign-in provider.
async fn share(context: &Context, args: ShareProjectArgs) -> Outcome {
    use limina_domain::collaboration::{CollaborationService, DirectoryPerson, Role};

    let db = crate::client::local::connect(&super::database_url(&context.database)).await?;
    db.initialize().await?;
    let collaboration = CollaborationService::new(db.clone());
    let operator = limina_core::Principal::local(&context.actor, true);
    let role = Role::parse(args.role.id())?;
    let shared = match args.subject {
        Some(subject) => {
            let person = DirectoryPerson {
                subject,
                display_name: args.name.unwrap_or_else(|| args.email.clone()),
                email: args.email.clone(),
            };
            collaboration.share_with_person(&args.project, &person, role, &operator).await
        }
        None => {
            collaboration.share_with_email(&args.project, &args.email, role, None, &operator).await
        }
    };
    db.close().await;
    let shared = serde_json::to_value(shared?).map_err(limina_core::internal)?;
    let project = args.project;
    context.out.emit(&shared, |value| {
        let role = field(value, "role");
        if value["pending"] == Value::Bool(true) {
            format!(
                "Shared {project} with {} as {role}; it applies the first time they sign in \
                 with this verified email.",
                field(value, "email")
            )
        } else {
            format!("Shared {project} with {} as {role}.", field(value, "display_name"))
        }
    });
    Ok(())
}

/// Reads a `kb/` folder straight into the database (`--database`), like `db upgrade`.
async fn import(context: &Context, args: ImportProjectArgs) -> Outcome {
    use limina_domain::import::{ImportOptions, import_knowledge_base};
    use limina_domain::project::ProjectOwner;
    use limina_domain::service::ChallengeService;

    let kb = if args.path.join("mission").join("CHALLENGE.md").is_file() {
        args.path.clone()
    } else {
        args.path.join("kb")
    };
    let folder = std::fs::canonicalize(&args.path).unwrap_or_else(|_| args.path.clone());
    let slug = args.slug.clone().unwrap_or_else(|| {
        let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let name = if name == "kb" {
            folder
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            name
        };
        slugify(&name)
    });
    let db = crate::client::local::connect(&super::database_url(&context.database)).await?;
    db.initialize().await?;
    let service = ChallengeService::new(db.clone(), None);
    let owner = args.owner.map(|subject| ProjectOwner {
        subject,
        display_name: args.owner_name,
        email: args.owner_email,
    });
    let report = import_knowledge_base(
        &service,
        &kb,
        ImportOptions {
            slug,
            name: args.name,
            runtime_engine: Some(args.runtime.id().to_owned()),
            owner,
            actor: context.actor.clone(),
        },
    )
    .await;
    db.close().await;
    let report = serde_json::to_value(report?).map_err(limina_core::internal)?;
    context.out.emit(&report, |value| {
        let mut lines = vec![format!(
            "Imported {} as a draft: {} hypotheses, {} experiments, {} findings.",
            field(value, "slug"),
            field(value, "hypotheses"),
            field(value, "experiments"),
            field(value, "findings")
        )];
        for skipped in value["skipped"].as_array().into_iter().flatten() {
            lines.push(format!("Skipped {}: {}", field(skipped, "path"), field(skipped, "reason")));
        }
        for note in value["notes"].as_array().into_iter().flatten() {
            lines.push(format!("Note: {}", note.as_str().unwrap_or_default()));
        }
        lines.join("\n")
    });
    Ok(())
}

/// Lowercase letters and digits separated by single hyphens.
pub fn slugify(value: &str) -> String {
    let mut slug = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_owned()
}

async fn create(context: &Context, args: CreateProjectArgs) -> Outcome {
    let mission = match args.mission.filter(|value| !value.is_empty()) {
        Some(mission) => mission,
        None => prompt("Mission").await?,
    };
    let success = match args.success.filter(|value| !value.is_empty()) {
        Some(success) => success,
        None => prompt("Success criteria").await?,
    };
    let name = match args.name.filter(|value| !value.is_empty()) {
        Some(name) => name,
        None => title_case(&args.slug.replace('-', " ")),
    };
    let payload = json!({
        "slug": args.slug,
        "name": name,
        "objective": mission,
        "success_criteria": success,
        "context": args.context,
        "runtime": args.runtime.id(),
    });
    let id = command_id();
    let result =
        context.public().create_project(&payload, Command::by(&context.actor, &id)).await?;
    context.out.emit(&result, render_project);
    Ok(())
}

/// `start`, `pause`, `resume`, `stop` and `project archive`.
pub async fn lifecycle(context: &Context, project: &str, action: &str) -> Outcome {
    let id = command_id();
    let result =
        context.public().project_action(project, action, Command::by(&context.actor, &id)).await?;
    context.out.emit(&result, render_project);
    Ok(())
}

pub async fn status(context: &Context, project: &str) -> Outcome {
    let result = context.public().project_status(project).await?;
    context.out.emit(&result, |value| {
        let mut text = value.get("project").map(render_project).unwrap_or_default();
        let mut rows = Vec::new();
        if let Some(Value::Object(kinds)) = value.get("knowledge") {
            let mut kinds: Vec<_> = kinds.iter().collect();
            kinds.sort_by(|a, b| a.0.cmp(b.0));
            for (kind, states) in kinds {
                if let Value::Object(states) = states {
                    let mut states: Vec<_> = states.iter().collect();
                    states.sort_by(|a, b| a.0.cmp(b.0));
                    for (state, count) in states {
                        rows.push(vec![kind.clone(), state.clone(), display(count)]);
                    }
                }
            }
        }
        if !rows.is_empty() {
            text.push('\n');
            text.push_str(&table("Knowledge", &["Kind", "State", "Count"], &rows));
        }
        text.push('\n');
        text.push_str(&format!("Pending team guidance: {}", field(value, "pending_guidance")));
        text
    });
    Ok(())
}

pub async fn review(context: &Context, args: &ReviewArgs) -> Outcome {
    if let Some(artifact) = args.artifact.as_deref().filter(|value| !value.is_empty()) {
        let result = context.public().knowledge(&args.project, &artifact.to_uppercase()).await?;
        context.out.emit(&result, |value| {
            let mut text = panel(
                &field(value, "id"),
                &format!("{}\nStatus  {}", field(value, "title"), field(value, "status")),
            );
            if let Some(Value::Object(content)) = value.get("content") {
                for (key, content) in content {
                    if truthy(content) {
                        text.push_str(&format!(
                            "\n{}\n{}\n",
                            title_case(&key.replace('_', " ")),
                            display(content)
                        ));
                    }
                }
            }
            text
        });
        return Ok(());
    }
    let result = context.public().review(&args.project).await?;
    context.out.emit(&result, |value| {
        let mut text = value.get("project").map(render_project).unwrap_or_default();
        for (title, key) in
            [("Findings", "findings"), ("Experiments", "experiments"), ("Hypotheses", "hypotheses")]
        {
            let rows: Vec<Vec<String>> = items(value.get(key).unwrap_or(&Value::Null))
                .iter()
                .map(|item| vec![field(item, "id"), field(item, "status"), field(item, "title")])
                .collect();
            if !rows.is_empty() {
                text.push('\n');
                text.push_str(&table(title, &["ID", "Status", "Title"], &rows));
            }
        }
        text
    });
    Ok(())
}

pub async fn steer(context: &Context, args: &SteerArgs) -> Outcome {
    let payload = json!({"body": args.message, "kind": args.kind});
    let id = command_id();
    let result = context
        .public()
        .steer_project(&args.project, &payload, Command::by(&context.actor, &id))
        .await?;
    context.out.emit(&result, |value| {
        format!("{} — Limina accepted your guidance.", title_case(&field(value, "delivery")))
    });
    Ok(())
}

/// Follows the activity stream until interrupted (or once with `--no-follow`).
pub async fn watch(context: &Context, args: &WatchArgs) -> Outcome {
    let follow = async {
        let mut cursor = args.after;
        loop {
            let result = context.public().activity(&args.project, cursor, 200).await?;
            for event in items(result.get("events").unwrap_or(&Value::Null)) {
                if context.out.as_json {
                    output::line(&crate::pyjson::dumps(event));
                } else {
                    output::line(&render_activity(event));
                }
            }
            if let Some(next) = result.get("cursor").and_then(Value::as_i64) {
                cursor = next;
            }
            if !args.following() {
                return Ok(());
            }
            let interval = Duration::try_from_secs_f64(args.interval.max(0.1))
                .unwrap_or(Duration::from_secs(1));
            tokio::time::sleep(interval).await;
        }
    };
    tokio::select! {
        result = follow => result,
        _ = tokio::signal::ctrl_c() => Ok(()),
    }
}

pub async fn export(context: &Context, args: &ExportArgs) -> Outcome {
    let files = context.public().snapshot(&args.project).await?;
    let files: Vec<(String, String)> =
        files.into_iter().map(|(path, content)| (path, display(&content))).collect();
    let written = write_snapshot(files, &args.target).await?;
    let target = std::fs::canonicalize(&args.target)
        .or_else(|_| std::path::absolute(&args.target))
        .unwrap_or_else(|_| args.target.clone());
    let result = json!({"target": target.display().to_string(), "file_count": written.len()});
    context.out.emit(&result, |value| {
        format!("Exported {} files to {}", field(value, "file_count"), field(value, "target"))
    });
    Ok(())
}

pub async fn resource(context: &Context, command: ResourceCommand) -> Outcome {
    match command {
        ResourceCommand::Variable(args) => {
            let id = command_id();
            let result = context
                .public()
                .set_variable(
                    &args.project,
                    &args.name,
                    &args.value,
                    Command::by(&context.actor, &id),
                )
                .await?;
            context.out.emit(&result, |item| {
                format!("Set variable {}={}", field(item, "name"), field(item, "value"))
            });
            Ok(())
        }
        ResourceCommand::Secret(args) => {
            let value = secret_value(&args).await?;
            let id = command_id();
            let result = context
                .public()
                .set_secret(&args.project, &args.name, &value, Command::by(&context.actor, &id))
                .await?;
            context.out.emit(&result, |item| {
                format!("Stored secret {} · value hidden", field(item, "name"))
            });
            Ok(())
        }
        ResourceCommand::List(args) => {
            let result = context.public().resources(&args.project).await?;
            context.out.emit(&result, |values| {
                let rows: Vec<Vec<String>> = items(values)
                    .iter()
                    .map(|item| {
                        let kind = field(item, "type");
                        let shown = if kind == "VARIABLE" {
                            field(item, "value")
                        } else if item.get("configured").is_some_and(truthy) {
                            "configured · hidden".to_owned()
                        } else {
                            "unavailable".to_owned()
                        };
                        vec![field(item, "name"), kind, shown]
                    })
                    .collect();
                table("Project resources", &["Name", "Type", "Value"], &rows)
            });
            Ok(())
        }
        ResourceCommand::Remove(args) => {
            let id = command_id();
            let result = context
                .public()
                .remove_resource(&args.project, &args.name, Command::by(&context.actor, &id))
                .await?;
            context.out.emit(&result, |value| format!("Removed {}", field(value, "name")));
            Ok(())
        }
    }
}

/// The secret from `--from-env`, `--from-stdin` (one trailing newline removed) or a hidden,
/// confirmed prompt.
async fn secret_value(args: &SecretArgs) -> Result<String, Failure> {
    const COMMAND: &[&str] = &["resource", "secret"];
    if args.from_env.is_some() && args.from_stdin {
        return Err(Failure::Usage {
            command: COMMAND,
            message: "Invalid value: choose either --from-env or --from-stdin".to_owned(),
        });
    }
    if let Some(name) = &args.from_env {
        return std::env::var(name).map_err(|_| Failure::Usage {
            command: COMMAND,
            message: format!(
                "Invalid value for --from-env: environment variable '{name}' is not set"
            ),
        });
    }
    if args.from_stdin {
        let mut value = blocking(|| {
            let mut value = String::new();
            std::io::stdin().read_to_string(&mut value).map(|_| value)
        })
        .await?;
        if value.ends_with('\n') {
            value.pop();
            if value.ends_with('\r') {
                value.pop();
            }
        }
        return Ok(value);
    }
    loop {
        let value = blocking(|| rpassword::prompt_password("Secret value: ")).await?;
        let confirmation =
            blocking(|| rpassword::prompt_password("Repeat for confirmation: ")).await?;
        if value == confirmation {
            return Ok(value);
        }
        output::error_line("Error: The two entered values do not match.");
    }
}

/// `typer.prompt(label)`: asks until a non-empty answer arrives. Prompts go to standard error so
/// `--json` output stays parseable.
async fn prompt(label: &'static str) -> Result<String, Failure> {
    loop {
        let answer = blocking(move || {
            eprint!("{label}: ");
            let mut line = String::new();
            let read = std::io::stdin().lock().read_line(&mut line)?;
            if read == 0 {
                return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
            }
            Ok(line.trim_end_matches(['\n', '\r']).to_owned())
        })
        .await?;
        if !answer.is_empty() {
            return Ok(answer);
        }
    }
}

/// Runs blocking terminal I/O off the async runtime. End of input aborts the command.
async fn blocking<T, F>(work: F) -> Result<T, Failure>
where
    T: Send + 'static,
    F: FnOnce() -> std::io::Result<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(_)) | Err(_) => Err(Failure::Aborted),
    }
}

/// The items of a JSON array (an empty slice for anything else).
fn items(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_lines_follow_the_python_layout() {
        let event = json!({
            "sequence": 7,
            "type": "hypothesis.created",
            "actor": "Limina",
            "artifact_id": "H001",
            "detail": {"title": "A claim"},
            "created_at": "2026-01-01T00:00:00.123456+00:00",
        });
        assert_eq!(
            render_activity(&event),
            "    7 2026-01-01T00:00:00 hypothesis.created H001 by Limina A claim"
        );
    }

    #[test]
    fn project_cards_show_the_engine_label() {
        let card = render_project(&json!({
            "slug": "demo", "name": "Demo", "mission": "M", "runtime": "claude-code",
            "status": "CREATED", "current_objective": "O", "next_step": "N", "blocker": "",
        }));
        assert!(card.starts_with("demo\n────\nDemo\nM\n\nEngine     Claude\n"));
    }
}
