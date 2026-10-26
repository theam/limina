//! Project creation, lifecycle and resources.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod service_support;

use limina_domain::project::{NewChallenge, ProjectOwner};
use limina_domain::service::ChallengeService;
use limina_store::{Db, args};
use service_support::{code, command_id, each_backend, retrieval};

const STATUSES: [&str; 7] =
    ["CREATED", "RUNNING", "WAITING", "PAUSED", "STOPPED", "COMPLETE", "FAILED"];

/// The Python runtime's `PROJECT_TRANSITIONS`.
fn python_transition(action: &str, status: &str) -> Option<&'static str> {
    match (action, status) {
        ("start", "CREATED" | "STOPPED")
        | ("resume", "PAUSED" | "WAITING" | "STOPPED" | "FAILED") => Some("RUNNING"),
        ("pause", "RUNNING" | "WAITING") => Some("PAUSED"),
        ("stop", "CREATED" | "RUNNING" | "WAITING" | "PAUSED" | "FAILED") => Some("STOPPED"),
        _ => None,
    }
}

async fn force_status(db: &Db, status: &str) {
    let mut tx = db.begin_write().await.unwrap();
    tx.execute(
        "UPDATE coordinator_states SET status = $1, worker_id = 'worker', wake_at = $2",
        args![status, limina_core::now()],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

async fn event_types(service: &ChallengeService) -> Vec<String> {
    service.events("retrieval", 0, 1000).await.unwrap().into_iter().map(|e| e.event_type).collect()
}

#[tokio::test]
async fn lifecycle_actions_follow_the_transition_table() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        for action in ["start", "pause", "resume", "stop"] {
            for status in STATUSES {
                force_status(&backend.db, status).await;
                let before = service.get_challenge("retrieval").await.unwrap();
                let events = event_types(&service).await.len();
                let outcome =
                    service.change_project_state("retrieval", action, "owner", &command_id()).await;
                let label = format!("{action} from {status}");
                let no_op = matches!(
                    (action, status),
                    ("start" | "resume", "RUNNING") | ("pause", "PAUSED") | ("stop", "STOPPED")
                );
                match python_transition(action, status) {
                    Some(target) => {
                        let after = outcome.unwrap();
                        let coordinator = after.coordinator.unwrap();
                        assert_eq!(coordinator.status, target, "{label}");
                        assert_eq!(coordinator.worker_id, None, "{label}");
                        assert_eq!(coordinator.wake_at, None, "{label}");
                        assert_eq!(coordinator.version, before.coordinator.unwrap().version + 1);
                        assert!(after.updated_at >= before.updated_at);
                        let types = event_types(&service).await;
                        assert_eq!(types.len(), events + 1, "{label}");
                        let expected_event = match action {
                            "start" => "project.started",
                            "pause" => "project.paused",
                            "resume" => "project.resumed",
                            _ => "project.stopped",
                        };
                        assert_eq!(types.last().unwrap(), expected_event);
                    }
                    None if no_op => {
                        assert_eq!(outcome.unwrap(), before, "{label} changes nothing");
                        assert_eq!(event_types(&service).await.len(), events, "{label}");
                    }
                    None => {
                        let error = outcome.unwrap_err();
                        assert_eq!(error.code(), "invariant_violation", "{label}");
                        assert_eq!(
                            error.message,
                            format!("Cannot {action} a project while it is {status}.")
                        );
                        assert_eq!(error.details["action"], action);
                        assert_eq!(error.details["status"], status);
                    }
                }
            }
        }
    })
    .await;
}

#[tokio::test]
async fn archiving_requires_an_idle_project_and_is_final() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        for status in ["RUNNING", "WAITING"] {
            force_status(&backend.db, status).await;
            let error =
                service.change_project_state("retrieval", "archive", "owner", &command_id()).await;
            let error = error.unwrap_err();
            assert_eq!(error.message, "Pause or stop the project before archiving it.");
            assert_eq!(error.details["status"], status);
        }
        force_status(&backend.db, "COMPLETE").await;
        let archived = service
            .change_project_state("retrieval", "ARCHIVE", "owner", &command_id())
            .await
            .unwrap();
        assert_eq!(archived.status, "ARCHIVED");
        assert_eq!(archived.coordinator.as_ref().unwrap().status, "STOPPED");
        assert_eq!(event_types(&service).await.last().unwrap(), "project.archived");
        let again = service
            .change_project_state("retrieval", "archive", "owner", &command_id())
            .await
            .unwrap();
        assert_eq!(again, archived);
        for action in ["start", "pause", "resume", "stop"] {
            let error = service
                .change_project_state("retrieval", action, "owner", &command_id())
                .await
                .unwrap_err();
            assert_eq!(error.message, "An archived project cannot be restarted.");
            assert_eq!(error.details["action"], action);
            assert_eq!(error.details["status"], "ARCHIVED");
        }
        assert!(service.list_projects(false).await.unwrap().is_empty());
        assert_eq!(service.list_projects(true).await.unwrap().len(), 1);
        assert_eq!(
            code(service.change_project_state("missing", "start", "owner", &command_id()).await),
            "not_found"
        );
    })
    .await;
}

#[tokio::test]
async fn projects_are_created_once_with_an_owner_and_listed_oldest_first() {
    each_backend(|backend| async move {
        let service = backend.service();
        let new = |slug: &str| NewChallenge {
            model: None,
            reasoning_effort: None,
            slug: slug.into(),
            name: "  Second  ".into(),
            objective: "Objective.".into(),
            success_criteria: "Criteria.".into(),
            context: "  ".into(),
            runtime_engine: None,
            owner: Some(ProjectOwner {
                subject: "oidc|ada".into(),
                display_name: Some(String::new()),
                email: Some("ada@example.test".into()),
            }),
        };
        let first =
            service.create_challenge(new(" Zeta-Project "), "Ada", &command_id()).await.unwrap();
        assert_eq!(first.slug, "zeta-project");
        assert_eq!(first.name, "Second");
        assert_eq!(first.context, "");
        assert_eq!(first.status, "ACTIVE");
        assert_eq!(first.version, 1);
        let coordinator = first.coordinator.as_ref().unwrap();
        assert_eq!(coordinator.status, "CREATED");
        assert_eq!(coordinator.current_objective, "Objective.");
        assert_eq!(coordinator.next_step, "Frame the first falsifiable hypothesis.");
        assert_eq!(coordinator.blocker, "None");
        assert_eq!((coordinator.inbox_cursor, coordinator.version), (0, 1));
        assert_eq!(coordinator.heartbeat_at, None);

        let mut tx = backend.db.begin().await.unwrap();
        let member: (String, String, String, Option<String>, String) = tx
            .fetch_one(
                "SELECT subject, role, display_name, email, created_by FROM project_members",
                args![],
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(
            member,
            (
                "oidc|ada".to_owned(),
                "OWNER".to_owned(),
                "oidc|ada".to_owned(),
                Some("ada@example.test".to_owned()),
                "oidc|ada".to_owned()
            )
        );

        let duplicate = service.create_challenge(new("ZETA-project"), "Ada", &command_id()).await;
        let error = duplicate.unwrap_err();
        assert_eq!(error.message, "Challenge 'zeta-project' already exists.");
        assert_eq!(error.details["challenge"], "zeta-project");
        let second = service.create_challenge(new("alpha"), "Ada", &command_id()).await.unwrap();
        let listed: Vec<String> =
            service.list_projects(false).await.unwrap().into_iter().map(|p| p.slug).collect();
        assert_eq!(listed, ["zeta-project", "alpha"], "oldest first, not by name");
        assert_eq!(service.get_challenge("ALPHA").await.unwrap(), second);
        for slug in ["", "-", "a--b", "a_b", "über"] {
            let error =
                service.create_challenge(new(slug), "Ada", &command_id()).await.unwrap_err();
            assert_eq!(error.code(), "invariant_violation", "{slug:?}");
        }
    })
    .await;
}

#[tokio::test]
async fn resources_are_validated_reactivated_and_resolved_for_providers() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let variable = service
            .set_variable("retrieval", "dataset_url", "  s3://data  ", "owner", &command_id())
            .await
            .unwrap();
        assert_eq!(variable.name, "DATASET_URL");
        assert_eq!(variable.value.as_deref(), Some("  s3://data  "), "values are stored as given");
        assert_eq!(variable.configured, None);
        let removed = service
            .remove_resource("retrieval", "Dataset_Url", "owner", &command_id())
            .await
            .unwrap();
        assert_eq!((removed.status.as_str(), removed.value.as_deref()), ("REMOVED", None));
        assert!(service.list_resources("retrieval", true).await.unwrap().is_empty());
        assert_eq!(service.list_resources("retrieval", false).await.unwrap().len(), 1);
        let secret = service
            .set_secret("retrieval", "DATASET_URL", "s3://private", "admin", &command_id())
            .await
            .unwrap();
        assert_eq!(secret.id, variable.id, "a removed name is reactivated in place");
        assert_eq!((secret.resource_type.as_str(), secret.status.as_str()), ("SECRET", "ACTIVE"));
        assert_eq!(secret.created_by, "admin");
        let environment = service.resource_environment("retrieval").await.unwrap();
        assert_eq!(environment["DATASET_URL"], "s3://private");

        let error =
            service.remove_resource("retrieval", "PATH", "owner", &command_id()).await.unwrap_err();
        assert_eq!(error.message, "Resource name 'PATH' is reserved by the managed runtime.");
        let error = service
            .remove_resource("retrieval", "UNKNOWN", "owner", &command_id())
            .await
            .unwrap_err();
        assert_eq!(
            (error.code(), error.message.as_str()),
            ("not_found", "Active resource 'UNKNOWN' does not exist.")
        );
        let error = service
            .set_variable("retrieval", "BIG", &"é".repeat(16_385), "owner", &command_id())
            .await
            .unwrap_err();
        assert_eq!(error.message, "Resource variable exceeds the 32768-byte limit.");
        assert!(
            service
                .set_variable("retrieval", "EXACT", &"x".repeat(32_768), "owner", &command_id())
                .await
                .is_ok()
        );

        // Rows the write-time policy would reject still fail closed when read.
        let mut tx = backend.db.begin_write().await.unwrap();
        tx.execute("UPDATE project_resources SET value = NULL WHERE name = 'EXACT'", args![])
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let error = service.resource_environment("retrieval").await.unwrap_err();
        assert_eq!(error.message, "Variable 'EXACT' has no value.");
        let mut tx = backend.db.begin_write().await.unwrap();
        tx.execute(
            "UPDATE project_resources SET resource_type = 'SECRET', secret_ciphertext = '' \
             WHERE name = 'EXACT'",
            args![],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let error = service.resource_environment("retrieval").await.unwrap_err();
        assert_eq!(error.message, "Resource 'EXACT' has invalid secret state.");
        service.remove_resource("retrieval", "EXACT", "owner", &command_id()).await.unwrap();

        let other_key = ChallengeService::new(
            backend.db.clone(),
            Some(std::sync::Arc::new(limina_store::SecretCipher::ephemeral())),
        );
        let error = other_key.resource_environment("retrieval").await.unwrap_err();
        assert_eq!(error.message, "Secret 'DATASET_URL' cannot be decrypted.");
        let no_cipher = ChallengeService::new(backend.db.clone(), None);
        let error = no_cipher.resource_environment("retrieval").await.unwrap_err();
        assert_eq!(error.message, "This Limina instance has no secret-encryption provider.");
    })
    .await;
}
