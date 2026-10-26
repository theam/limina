//! Kickoff drafts, preflight, guidance history, runtime runs and analytics. Ported from
//! `tests/test_cloud_ui_readiness.py`, `tests/test_cloud_enterprise_hardening.py` and
//! `tests/test_cloud_api.py` at the service level.

#![allow(clippy::unwrap_used)]

mod collaboration_support;

use std::cell::RefCell;

use chrono::{Duration, TimeZone, Utc};
use collaboration_support::*;
use limina_core::{LiminaError, now};
use limina_domain::collaboration::{CheckStatus, DraftChanges, EngineReadiness, RunOutcome};
use limina_domain::knowledge::SourceInput;
use limina_domain::usage::RuntimeUsage;
use limina_store::args;
use serde_json::{Value, json};

fn keys(value: &Value) -> Vec<&str> {
    value.as_object().unwrap().keys().map(String::as_str).collect()
}

fn engine(ready: bool, detail: &str) -> EngineReadiness {
    EngineReadiness {
        label: "Codex".to_owned(),
        installed: true,
        credentials: ready,
        detail: detail.to_owned(),
    }
}

/// The kickoff part of `test_kickoff_knowledge_sources_runs_and_analytics_are_queryable`.
#[tokio::test]
async fn kickoff_fields_change_only_before_the_first_start() {
    for backend in backends().await {
        let service = backend.service();
        let id = create_project(&backend.db, "ui-ready", Some(&owner())).await;

        let context = DraftChanges {
            context: Some(" A richer kickoff context. ".to_owned()),
            ..DraftChanges::default()
        };
        let updated = service.update_draft("ui-ready", &context, "Olivia Owner").await.unwrap();
        assert_eq!(updated.context, "A richer kickoff context.");
        assert_eq!((updated.version, updated.runtime_engine.as_str()), (2, "codex"));
        let json = serde_json::to_value(&updated).unwrap();
        assert_eq!(
            keys(&json),
            [
                "id",
                "slug",
                "name",
                "objective",
                "context",
                "success_criteria",
                "runtime_engine",
                "status",
                "version",
                "created_at",
                "updated_at",
                "coordinator"
            ]
        );
        assert_eq!(
            keys(&json["coordinator"]),
            ["status", "current_objective", "next_step", "blocker", "updated_at"]
        );

        let mission = DraftChanges {
            name: Some(" Renamed ".to_owned()),
            mission: Some("Find the strongest reranker.".to_owned()),
            success_criteria: Some("A reproducible win.".to_owned()),
            runtime: Some(" Claude-Code ".to_owned()),
            ..DraftChanges::default()
        };
        let updated = service.update_draft("UI-READY", &mission, "Olivia Owner").await.unwrap();
        assert_eq!(updated.name, "Renamed");
        assert_eq!(updated.objective, "Find the strongest reranker.");
        assert_eq!(updated.coordinator.current_objective, "Find the strongest reranker.");
        assert_eq!(updated.coordinator.next_step, "Frame the first falsifiable hypothesis.");
        assert_eq!(updated.context, "A richer kickoff context.", "unchanged fields are kept");
        assert_eq!((updated.version, updated.runtime_engine.as_str()), (3, "claude-code"));

        for (changes, message) in [
            (
                DraftChanges { name: Some(" ".to_owned()), ..DraftChanges::default() },
                "Name cannot be empty.",
            ),
            (
                DraftChanges { mission: Some(String::new()), ..DraftChanges::default() },
                "Mission cannot be empty.",
            ),
            (
                DraftChanges { success_criteria: Some("\n".to_owned()), ..DraftChanges::default() },
                "Success criteria cannot be empty.",
            ),
            (
                DraftChanges { runtime: Some("other".to_owned()), ..DraftChanges::default() },
                "Runtime engine must be one of: claude-code, codex.",
            ),
        ] {
            let error =
                service.update_draft("ui-ready", &changes, "Olivia Owner").await.unwrap_err();
            assert_eq!((error.code(), error.message.as_str()), ("invariant_violation", message));
            assert!(error.details.is_empty(), "{message}");
        }

        let mut tx = backend.db.begin_write().await.unwrap();
        tx.execute("UPDATE coordinator_states SET status = 'RUNNING'", args![]).await.unwrap();
        tx.commit().await.unwrap();
        let started = service.update_draft("ui-ready", &context, "Olivia Owner").await.unwrap_err();
        assert_eq!(
            started.message,
            "Project kickoff fields can only change before the first start."
        );

        let events: Vec<_> = event_log(&backend.db, &id).await;
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0],
            ("project.draft_updated".to_owned(), "Olivia Owner".to_owned(), None, json!({}))
        );
    }
}

/// `test_preflight_runtime_check_requires_engine_credentials` and the preflight part of
/// `test_kickoff_knowledge_sources_runs_and_analytics_are_queryable`.
#[tokio::test]
async fn preflight_reports_mission_criteria_engine_and_sources() {
    for backend in backends().await {
        let service = backend.service();
        create_project(&backend.db, "ui-ready", Some(&owner())).await;
        let probed = RefCell::new(Vec::new());

        let missing = service
            .preflight("ui-ready", |name| {
                probed.borrow_mut().push(name);
                async { Ok(engine(false, "Sign in with ChatGPT to run Codex turns.")) }
            })
            .await
            .unwrap();
        assert!(missing.ready, "a warning does not block the kickoff");
        assert_eq!(
            serde_json::to_value(&missing).unwrap(),
            json!({"ready": true, "checks": [
                {"name": "mission", "status": "PASS", "detail": "Mission is defined."},
                {"name": "success_criteria", "status": "PASS", "detail": "Success criteria are defined."},
                {"name": "runtime", "status": "WARN", "detail": "Sign in with ChatGPT to run Codex turns."},
                {"name": "sources", "status": "INFO", "detail": "0 source(s) registered."},
            ]})
        );

        let source = SourceInput {
            name: "benchmark".to_owned(),
            source_type: "URL".to_owned(),
            uri: "https://example.test/benchmark".to_owned(),
            ..SourceInput::default()
        };
        service.set_source("ui-ready", &source, "Olivia Owner").await.unwrap();
        let ready = service
            .preflight("ui-ready", |_| async {
                Ok(engine(true, "Signed in with ChatGPT (owner@example.test)"))
            })
            .await
            .unwrap();
        assert_eq!(ready.checks[2].status, CheckStatus::Pass);
        assert_eq!(
            ready.checks[2].detail,
            "Codex is ready. Signed in with ChatGPT (owner@example.test)"
        );
        assert_eq!(
            (ready.checks[3].status, ready.checks[3].detail.as_str()),
            (CheckStatus::Pass, "1 source(s) registered.")
        );

        // The probe receives the project's engine, and runs after the database work.
        let claude =
            DraftChanges { runtime: Some("claude-code".to_owned()), ..DraftChanges::default() };
        service.update_draft("ui-ready", &claude, "Olivia Owner").await.unwrap();
        service
            .preflight("ui-ready", |name| {
                probed.borrow_mut().push(name);
                async { Ok(engine(false, "Connect an account.")) }
            })
            .await
            .unwrap();
        assert_eq!(*probed.borrow(), ["codex", "claude-code"]);

        let mut tx = backend.db.begin_write().await.unwrap();
        tx.execute("UPDATE challenges SET objective = ' ', success_criteria = ''", args![])
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let empty =
            service.preflight("ui-ready", |_| async { Ok(engine(true, "Ready.")) }).await.unwrap();
        assert!(!empty.ready);
        assert_eq!(
            (empty.checks[0].status, empty.checks[0].detail.as_str()),
            (CheckStatus::Fail, "Mission is empty.")
        );
        assert_eq!(empty.checks[1].detail, "Success criteria are empty.");

        let failed = service
            .preflight("ui-ready", |_| async { Err(LiminaError::transport("probe failed")) })
            .await
            .unwrap_err();
        assert_eq!(failed.message, "probe failed");
    }
}

/// The guidance-history part of `test_kickoff_knowledge_sources_runs_and_analytics_are_queryable`.
#[tokio::test]
async fn guidance_history_pages_newest_first() {
    for backend in backends().await {
        let service = backend.service();
        let id = create_project(&backend.db, "ui-ready", Some(&owner())).await;
        let first = insert_guidance(
            &backend.db,
            &id,
            "Start with the dev split.",
            "ACKNOWLEDGED",
            at(1, 9, 0),
            Some(at(1, 9, 5)),
        )
        .await;
        insert_guidance(&backend.db, &id, "Compare two rerankers.", "PENDING", at(1, 10, 0), None)
            .await;
        let newest = insert_guidance(
            &backend.db,
            &id,
            "Prioritize the strongest evidence.",
            "ACKNOWLEDGED",
            at(1, 11, 0),
            Some(at(1, 11, 7)),
        )
        .await;

        let page = service.guidance("ui-ready", None, None, 2).await.unwrap();
        assert_eq!((page.total, page.next_cursor.as_deref()), (3, Some("Mg")));
        assert_eq!(page.items[0].id, newest);
        assert_eq!(page.items[0].body, "Prioritize the strongest evidence.");
        assert_eq!(page.items[0].status, "ACKNOWLEDGED");
        let json = serde_json::to_value(&page).unwrap();
        assert_eq!(keys(&json), ["items", "next_cursor", "total"]);
        assert_eq!(
            json["items"][1],
            json!({
                "id": page.items[1].id, "sequence": page.items[1].sequence, "kind": "STEER",
                "body": "Compare two rerankers.", "actor": "Olivia Owner", "status": "PENDING",
                "created_at": "2026-10-01T10:00:00+00:00", "acknowledged_at": null
            })
        );

        let rest =
            service.guidance("ui-ready", None, page.next_cursor.as_deref(), 2).await.unwrap();
        assert_eq!(
            (rest.items.len(), rest.items[0].id.as_str(), rest.next_cursor),
            (1, first.as_str(), None)
        );
        let acknowledged =
            service.guidance("ui-ready", Some("acknowledged"), None, 50).await.unwrap();
        assert_eq!(acknowledged.total, 2);
        assert_eq!(
            service.guidance("ui-ready", Some(""), None, 0).await.unwrap().items.len(),
            1,
            "limit clamps to 1"
        );
        let invalid = service.guidance("ui-ready", None, Some("M"), 50).await.unwrap_err();
        assert_eq!(
            (invalid.code(), invalid.message.as_str()),
            ("invariant_violation", "Pagination cursor is invalid.")
        );
    }
}

/// The runs part of `test_kickoff_knowledge_sources_runs_and_analytics_are_queryable`.
#[tokio::test]
async fn runs_record_usage_tool_calls_and_events() {
    for backend in backends().await {
        let service = backend.service();
        let id = create_project(&backend.db, "ui-ready", Some(&owner())).await;
        let run_id = service.start_run("ui-ready", "codex", Some("test-model"), 0).await.unwrap();
        let running = service.runs("ui-ready", None, None, 50).await.unwrap();
        assert_eq!((running.total, running.items[0].status.as_str()), (1, "RUNNING"));
        assert_eq!(running.items[0].completed_at, None);

        service.note_run_event(&run_id, false).await.unwrap();
        service.note_run_event(&run_id, true).await.unwrap();
        insert_event(
            &backend.db,
            &id,
            "runtime.codex",
            "limina:worker",
            json!({"run_id": run_id, "text": "Updating durable project knowledge"}),
        )
        .await;
        insert_event(&backend.db, &id, "runtime.codex", "Olivia Owner", json!({"run_id": run_id}))
            .await;
        insert_event(
            &backend.db,
            &id,
            "runtime.codex",
            "limina:worker",
            json!({"run_id": "another-run"}),
        )
        .await;
        insert_event(&backend.db, &id, "project.created", "Olivia Owner", json!({})).await;

        let outcome = RunOutcome {
            status: "COMPLETED".to_owned(),
            summary: "Evidence checkpoint completed.".to_owned(),
            turn_id: Some("turn-1".to_owned()),
            usage: RuntimeUsage {
                input_tokens: Some(120),
                output_tokens: Some(30),
                cached_input_tokens: Some(10),
                cost_microusd: Some(500),
                usage_source: Some("provider".to_owned()),
                ..RuntimeUsage::default()
            },
            ..RunOutcome::default()
        };
        service.finish_run(&run_id, &outcome).await.unwrap();
        service.finish_run("unknown-run", &outcome).await.unwrap();

        let runs = service.runs("ui-ready", None, None, 50).await.unwrap();
        let run = &runs.items[0];
        assert_eq!(
            (run.status.as_str(), run.usage.input_tokens, run.tool_calls),
            ("COMPLETED", Some(120), 1)
        );
        assert!(run.duration_ms.unwrap() >= 0 && run.completed_at.is_some());
        assert_eq!(run.error, None);
        let json = serde_json::to_value(run).unwrap();
        assert_eq!(
            keys(&json),
            [
                "id",
                "runtime",
                "model",
                "status",
                "summary",
                "error",
                "usage",
                "tool_calls",
                "retry_count",
                "started_at",
                "completed_at",
                "duration_ms"
            ]
        );
        assert_eq!(
            json["usage"],
            json!({
                "input_tokens": 120, "output_tokens": 30, "cached_input_tokens": 10,
                "reasoning_output_tokens": null, "total_tokens": null, "cost_microusd": 500,
                "usage_source": "provider", "cost_source": null
            })
        );

        let detail = service.run("ui-ready", &run_id).await.unwrap();
        let json = serde_json::to_value(&detail).unwrap();
        assert_eq!(keys(&json).last(), Some(&"events"));
        assert_eq!(json["id"], run_id.as_str());
        let events = json["events"].as_array().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(
            keys(&events[0]),
            ["sequence", "type", "actor", "artifact_id", "payload", "created_at"]
        );
        assert_eq!(
            (events[0]["actor"].as_str(), events[1]["actor"].as_str()),
            (Some("Limina"), Some("Olivia Owner"))
        );
        assert!(events[0]["sequence"].as_i64() < events[1]["sequence"].as_i64());

        let missing = service.run("ui-ready", "unknown-run").await.unwrap_err();
        assert_eq!(
            (missing.code(), missing.message.as_str()),
            ("not_found", "Runtime run does not exist.")
        );

        let failed_id = service.start_run("ui-ready", "claude-code", None, 0).await.unwrap();
        let failure = RunOutcome {
            status: "FAILED".to_owned(),
            error_code: Some("transport_error".to_owned()),
            error_message: Some("provider overloaded".to_owned()),
            ..RunOutcome::default()
        };
        service.finish_run(&failed_id, &failure).await.unwrap();
        let failed = service.run("ui-ready", &failed_id).await.unwrap();
        assert_eq!(
            serde_json::to_value(&failed.run.error).unwrap(),
            json!({"code": "transport_error", "message": "provider overloaded"})
        );
        assert!(failed.events.is_empty());
        let only_failed = service.runs("ui-ready", Some("failed"), None, 50).await.unwrap();
        assert_eq!((only_failed.total, only_failed.items[0].id.as_str()), (1, failed_id.as_str()));
        let page = service.runs("ui-ready", None, None, 1).await.unwrap();
        assert_eq!(page.next_cursor.as_deref(), Some("MQ"));
        let invalid = service.runs("ui-ready", None, Some("-1"), 1).await.unwrap_err();
        assert_eq!(invalid.message, "Pagination cursor is invalid.");
    }
}

/// `test_retry_attempts_have_distinct_runs_and_clear_wake_state` (the run records).
#[tokio::test]
async fn retries_have_distinct_runs() {
    for backend in backends().await {
        let service = backend.service();
        create_project(&backend.db, "retry-project", Some(&owner())).await;
        assert_eq!(service.next_retry_count("retry-project").await.unwrap(), 0);

        let first = service.start_run("retry-project", "codex", Some("test"), 0).await.unwrap();
        let failure = RunOutcome { status: "FAILED".to_owned(), ..RunOutcome::default() };
        service.finish_run(&first, &failure).await.unwrap();
        let retry = service.next_retry_count("retry-project").await.unwrap();
        assert_eq!(retry, 1);
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let second =
            service.start_run("retry-project", "codex", Some("test"), retry).await.unwrap();
        let success = RunOutcome {
            status: "COMPLETED".to_owned(),
            summary: "Recovered.".to_owned(),
            ..RunOutcome::default()
        };
        service.finish_run(&second, &success).await.unwrap();

        let runs = service.runs("retry-project", None, None, 50).await.unwrap().items;
        let statuses: Vec<&str> = runs.iter().map(|run| run.status.as_str()).collect();
        let retries: Vec<i32> = runs.iter().map(|run| run.retry_count).collect();
        assert_eq!((statuses, retries), (vec!["COMPLETED", "FAILED"], vec![1, 0]));
        assert_eq!(service.next_retry_count("retry-project").await.unwrap(), 0);
    }
}

/// The analytics part of `test_kickoff_knowledge_sources_runs_and_analytics_are_queryable`, with
/// every figure checked against the Python runtime's output for the same rows.
#[tokio::test]
async fn analytics_summarize_runs_knowledge_and_guidance() {
    for backend in backends().await {
        let service = backend.service();
        let id = create_project(&backend.db, "ui-ready", Some(&owner())).await;
        let today = now().date_naive();
        let day = |offset: i64, hour: u32| {
            Utc.from_utc_datetime(
                &(today - Duration::days(offset)).and_hms_opt(hour, 0, 0).unwrap(),
            )
        };
        let date = |offset: i64| (today - Duration::days(offset)).format("%Y-%m-%d").to_string();
        let run = |status, started_at, duration_ms| RunRow {
            status,
            started_at,
            duration_ms,
            input_tokens: None,
            output_tokens: None,
            reasoning_output_tokens: None,
            total_tokens: None,
            cost_microusd: None,
            usage_source: None,
            cost_source: None,
            tool_calls: 0,
        };
        // Inserted in start order, so the row order (and the order of count keys) is stable.
        insert_run(&backend.db, &id, run("COMPLETED", day(45, 9), Some(10))).await;
        insert_run(
            &backend.db,
            &id,
            RunRow {
                input_tokens: Some(100),
                output_tokens: Some(20),
                reasoning_output_tokens: Some(5),
                total_tokens: Some(125),
                cost_microusd: Some(300),
                usage_source: Some("provider"),
                cost_source: Some("provider"),
                tool_calls: 2,
                ..run("COMPLETED", day(3, 9), Some(1500))
            },
        )
        .await;
        insert_run(&backend.db, &id, run("FAILED", day(3, 10), Some(2500))).await;
        insert_run(
            &backend.db,
            &id,
            RunRow {
                input_tokens: Some(50),
                total_tokens: Some(60),
                usage_source: Some("provider"),
                cost_source: Some("operator_rate"),
                tool_calls: 1,
                ..run("COMPLETED", day(2, 9), Some(1000))
            },
        )
        .await;
        insert_run(&backend.db, &id, run("RUNNING", day(1, 0), None)).await;

        insert_artifact(
            &backend.db,
            &id,
            ArtifactRow::hypothesis("H001", "Old", json!({}), day(45, 9)),
        )
        .await;
        insert_artifact(
            &backend.db,
            &id,
            ArtifactRow::hypothesis("H002", "First", json!({}), day(3, 11)),
        )
        .await;
        insert_artifact(
            &backend.db,
            &id,
            ArtifactRow::hypothesis("H003", "Second", json!({}), day(2, 11)),
        )
        .await;
        insert_artifact(
            &backend.db,
            &id,
            ArtifactRow {
                kind: "E",
                status: "RUNNING",
                hypothesis: Some("H002"),
                ..ArtifactRow::hypothesis("E001", "Experiment", json!({}), day(2, 12))
            },
        )
        .await;

        let acknowledged =
            |at: chrono::DateTime<Utc>, micros: i64| Some(at + Duration::microseconds(micros));
        insert_guidance(
            &backend.db,
            &id,
            "a",
            "ACKNOWLEDGED",
            day(3, 12),
            acknowledged(day(3, 12), 90_500_000),
        )
        .await;
        insert_guidance(
            &backend.db,
            &id,
            "b",
            "ACKNOWLEDGED",
            day(2, 12),
            acknowledged(day(2, 12), 31_000_000),
        )
        .await;
        insert_guidance(&backend.db, &id, "c", "PENDING", day(1, 0), None).await;

        let analytics = service.analytics("ui-ready", 30).await.unwrap();
        let elapsed = now() - analytics.window.since;
        assert!(
            elapsed >= Duration::days(30) && elapsed < Duration::days(30) + Duration::seconds(5)
        );
        let mut json = serde_json::to_value(&analytics).unwrap();
        assert_eq!(keys(&json), ["window", "runs", "knowledge", "guidance", "timeseries"]);
        json["window"]["since"] = Value::Null;
        assert_eq!(
            json,
            json!({
                "window": {"days": 30, "since": null},
                "runs": {
                    "total": 4,
                    "by_status": {"COMPLETED": 2, "FAILED": 1, "RUNNING": 1},
                    "success_rate": 0.5,
                    "average_duration_ms": 1667,
                    "p95_duration_ms": 1500,
                    "input_tokens": 150,
                    "output_tokens": 20,
                    "reasoning_output_tokens": 5,
                    "total_tokens": 185,
                    "cost_microusd": 300,
                    "usage_sources": ["provider"],
                    "cost_sources": ["operator_rate", "provider"],
                    "tool_calls": 3
                },
                "knowledge": {"created": 3, "by_kind": {"H": 2, "E": 1}, "by_status": {"PROPOSED": 2, "RUNNING": 1}},
                "guidance": {"total": 3, "pending": 1, "average_acknowledgement_seconds": 60},
                "timeseries": [
                    {"date": date(3), "runs": 2, "completed_runs": 1, "failed_runs": 1, "artifacts": 1, "guidance": 1},
                    {"date": date(2), "runs": 1, "completed_runs": 1, "failed_runs": 0, "artifacts": 2, "guidance": 1},
                    {"date": date(1), "runs": 1, "completed_runs": 0, "failed_runs": 0, "artifacts": 0, "guidance": 1}
                ]
            })
        );
        assert_eq!(keys(&json["runs"])[..3], ["total", "by_status", "success_rate"]);
        assert_eq!(
            keys(&json["timeseries"][0]),
            ["date", "runs", "completed_runs", "failed_runs", "artifacts", "guidance"]
        );

        // The window is clamped to 1–365 days but reported as requested.
        let wide = service.analytics("ui-ready", 1000).await.unwrap();
        assert_eq!((wide.window.days, wide.runs.total, wide.knowledge.created), (1000, 5, 4));
        let narrow = service.analytics("ui-ready", 0).await.unwrap();
        assert_eq!((narrow.window.days, narrow.runs.total), (0, 0));
        assert_eq!(narrow.runs.success_rate, None);
        assert_eq!(narrow.runs.average_duration_ms, None);
        assert_eq!(narrow.runs.input_tokens, None);
        assert!(narrow.timeseries.is_empty());
    }
}

/// `test_domain_errors_have_stable_machine_readable_shape`
#[tokio::test]
async fn missing_projects_are_not_found() {
    for backend in backends().await {
        let service = backend.service();
        let errors = [
            service.list_members("missing").await.unwrap_err(),
            service.role_for("missing", &viewer()).await.unwrap_err(),
            service.guidance("missing", None, None, 50).await.unwrap_err(),
            service.runs("missing", None, None, 50).await.unwrap_err(),
            service.analytics("missing", 30).await.unwrap_err(),
            service.next_retry_count("missing").await.unwrap_err(),
            service.start_run("missing", "codex", None, 0).await.unwrap_err(),
        ];
        for error in errors {
            assert_eq!((error.code(), error.http_status()), ("not_found", 404));
            assert_eq!(error.message, "Project 'missing' does not exist.");
            assert_eq!(Value::Object(error.details), json!({"project": "missing"}));
        }
    }
}

#[tokio::test]
async fn model_defaults_persist_reset_and_change_after_kickoff() {
    for backend in backends().await {
        let service = backend.service();
        create_project(&backend.db, "defaults", Some(&owner())).await;
        let choice = DraftChanges {
            model: Some("project-model".into()),
            reasoning_effort: Some("ultra".into()),
            ..Default::default()
        };
        let saved = service.update_draft("defaults", &choice, "owner").await.unwrap();
        assert_eq!(saved.model.as_deref(), Some("project-model"));
        assert_eq!(saved.reasoning_effort.as_deref(), Some("ultra"));
        // A provider switch clears choices that belong to the previous provider.
        let switch = DraftChanges { runtime: Some("claude-code".into()), ..Default::default() };
        let saved = service.update_draft("defaults", &switch, "owner").await.unwrap();
        assert!(saved.model.is_none() && saved.reasoning_effort.is_none());
        let invalid =
            DraftChanges { reasoning_effort: Some("not a mode".into()), ..Default::default() };
        assert!(service.update_draft("defaults", &invalid, "owner").await.is_err());
        let mut tx = backend.db.begin_write().await.unwrap();
        tx.execute(
            "UPDATE coordinator_states SET status = 'PAUSED' WHERE challenge_id = $1",
            limina_store::args![&saved.id],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let choice = DraftChanges {
            model: Some("opus".into()),
            reasoning_effort: Some("max".into()),
            ..Default::default()
        };
        let saved = service.update_draft("defaults", &choice, "owner").await.unwrap();
        let long_model = "m".repeat(200);
        service.start_run("defaults", "claude-code", Some(&long_model), 0).await.unwrap();
        assert_eq!(saved.coordinator.status, "PAUSED");
        assert_eq!(saved.reasoning_effort.as_deref(), Some("max"));
        let reset = DraftChanges {
            model: Some(String::new()),
            reasoning_effort: Some(String::new()),
            ..Default::default()
        };
        let saved = service.update_draft("defaults", &reset, "owner").await.unwrap();
        assert!(saved.model.is_none() && saved.reasoning_effort.is_none());
    }
}
