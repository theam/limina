//! The service's results and errors match the Python runtime's JSON exactly: same keys, key order
//! and value types. `fixtures/shapes.json` holds what the Python service returned for this same
//! session (a frozen recording), with UUIDs, timestamps and dates replaced by placeholders.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::too_many_lines)]

mod service_support;

use chrono::TimeZone;
use limina_core::Result;
use limina_domain::project::{NewChallenge, ProjectOwner};
use limina_domain::research::{
    ExperimentOutcome, HypothesisDecision, NewExperiment, NewFinding, NewHypothesis,
};
use limina_domain::service::{ChallengeService, Checkpoint};
use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value, json};
use service_support::{command_id, each_backend};

const SLUG: &str = "shapes";
const HIDDEN: &[&str] = &["coordinator.claimed", "coordinator.released"];

struct Normalizer {
    uuid: Regex,
    timestamp: Regex,
    date: Regex,
}

impl Normalizer {
    fn new() -> Self {
        Self {
            uuid: Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
                .unwrap(),
            timestamp: Regex::new(r"^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(\.\d{1,6})?\+00:00$")
                .unwrap(),
            date: Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap(),
        }
    }

    fn text(&self, text: String) -> String {
        if self.uuid.is_match(&text) {
            "<uuid>".to_owned()
        } else if self.timestamp.is_match(&text) {
            "<timestamp>".to_owned()
        } else if self.date.is_match(&text) {
            "<date>".to_owned()
        } else {
            text
        }
    }

    fn value(&self, value: Value) -> Value {
        match value {
            Value::Object(map) => {
                Value::Object(map.into_iter().map(|(k, v)| (self.text(k), self.value(v))).collect())
            }
            Value::Array(items) => Value::Array(items.into_iter().map(|v| self.value(v)).collect()),
            Value::String(text) => Value::String(self.text(text)),
            other => other,
        }
    }
}

struct Session {
    service: ChallengeService,
    normalizer: Normalizer,
    results: Map<String, Value>,
}

impl Session {
    fn record<T: Serialize>(&mut self, name: &str, result: Result<T>) -> Option<T> {
        let (shape, value) = match result {
            Ok(value) => (serde_json::to_value(&value).unwrap(), Some(value)),
            Err(error) => (error.to_body(), None),
        };
        self.results.insert(name.to_owned(), self.normalizer.value(shape));
        value
    }

    async fn create(&mut self, name: &str, change: impl FnOnce(&mut NewChallenge)) {
        let mut new = NewChallenge {
            model: None,
            reasoning_effort: None,
            slug: format!(" {} ", SLUG.to_uppercase()),
            name: " Shapes ".into(),
            objective: " Pin every JSON shape. ".into(),
            success_criteria: "Rust matches Python.".into(),
            context: " Golden fixture. ".into(),
            runtime_engine: Some(" Claude-Code ".into()),
            owner: Some(ProjectOwner {
                subject: "owner-subject".into(),
                display_name: None,
                email: Some("owner@example.test".into()),
            }),
        };
        change(&mut new);
        let result = self.service.create_challenge(new, "owner", &command_id()).await;
        self.record(name, result);
    }

    fn checkpoint(change: impl FnOnce(&mut Checkpoint)) -> Checkpoint {
        let mut checkpoint = Checkpoint {
            current_objective: " Compare rerankers. ".into(),
            next_step: " Run the benchmark. ".into(),
            blocker: "  ".into(),
            status: "running".into(),
            worker_id: Some("worker".into()),
            continuation_id: Some("thread-1".into()),
            inbox_cursor: 1,
            expected_version: 1,
            acknowledge_message_ids: Vec::new(),
            wake_at: None,
        };
        change(&mut checkpoint);
        checkpoint
    }

    fn hypothesis(change: impl FnOnce(&mut NewHypothesis)) -> NewHypothesis {
        let mut new = NewHypothesis {
            title: " Reranking ".into(),
            statement: " A reranker improves NDCG. ".into(),
            mechanism: " Pairwise relevance. ".into(),
            generalization: String::new(),
            shortcut_risks: "Overfitting.".into(),
            test_plan: "Frozen candidates.".into(),
        };
        change(&mut new);
        new
    }

    fn experiment(change: impl FnOnce(&mut NewExperiment)) -> NewExperiment {
        let mut new = NewExperiment {
            hypothesis_id: "h001".into(),
            title: " Benchmark ".into(),
            objective: " Measure NDCG. ".into(),
            procedure: "Run both.".into(),
            success_criteria: "+10%.".into(),
            guardrails: String::new(),
        };
        change(&mut new);
        new
    }

    fn finding(change: impl FnOnce(&mut NewFinding)) -> NewFinding {
        let mut new = NewFinding {
            experiment_id: "e001".into(),
            title: " Reranking helps ".into(),
            finding: " It helps. ".into(),
            evidence: " +12% NDCG. ".into(),
            improvement: String::new(),
            remaining_debt: "Latency.".into(),
            next_move: "Measure online.".into(),
            impact: "high".into(),
        };
        change(&mut new);
        new
    }

    async fn change(&mut self, name: &str, action: &str) {
        let result = self.service.change_project_state(SLUG, action, "owner", &command_id()).await;
        self.record(name, result);
    }
}

/// The session the fixture was recorded from.
async fn run(service: ChallengeService) -> Map<String, Value> {
    let mut s =
        Session { service: service.clone(), normalizer: Normalizer::new(), results: Map::new() };
    let svc = service;

    s.create("create_challenge", |_| {}).await;
    s.create("create_challenge_duplicate", |new| new.slug = SLUG.into()).await;
    s.create("create_challenge_bad_slug", |new| new.slug = "Bad--Slug".into()).await;
    s.create("create_challenge_bad_engine", |new| {
        new.slug = "other".into();
        new.runtime_engine = Some("gpt".into());
    })
    .await;
    s.create("create_challenge_empty_name", |new| {
        new.slug = "other".into();
        new.name = " ".into();
    })
    .await;
    s.record("get_challenge", svc.get_challenge(SLUG).await);
    s.record("get_challenge_missing", svc.get_challenge("Missing").await);
    s.record("list_projects", svc.list_projects(false).await);
    s.record("claim_coordinator", svc.claim_coordinator(SLUG, 300, "worker", &command_id()).await);
    s.record(
        "claim_coordinator_taken",
        svc.claim_coordinator(SLUG, 300, "other", &command_id()).await,
    );
    s.record(
        "claim_coordinator_bad_ttl",
        svc.claim_coordinator(SLUG, 29, "worker", &command_id()).await,
    );
    let message = s
        .record(
            "send_message",
            svc.send_message(
                SLUG,
                "steer",
                " Prefer the frozen baseline. ",
                Some("resource"),
                "owner",
                &command_id(),
            )
            .await,
        )
        .unwrap();
    s.record(
        "send_message_bad_kind",
        svc.send_message(SLUG, "shout", "x", None, "owner", &command_id()).await,
    );
    s.record(
        "send_message_too_long",
        svc.send_message(SLUG, "STEER", &"é".repeat(32_769), None, "owner", &command_id()).await,
    );
    s.record("inbox", svc.inbox(SLUG, 0, false).await);
    s.record("inbox_pending_after", svc.inbox(SLUG, 1, true).await);
    s.record("message_bodies", svc.message_bodies(SLUG, [&message.id]).await);
    s.record("message_bodies_empty", svc.message_bodies("missing", Vec::<String>::new()).await);
    let ids = vec!["missing-b".to_owned(), "missing-a".to_owned(), message.id.clone()];
    s.record(
        "checkpoint_missing_message",
        svc.checkpoint_coordinator(
            SLUG,
            Session::checkpoint(|c| c.acknowledge_message_ids = ids),
            "worker",
            &command_id(),
        )
        .await,
    );
    s.record(
        "checkpoint_bad_status",
        svc.checkpoint_coordinator(
            SLUG,
            Session::checkpoint(|c| c.status = "sleeping".into()),
            "worker",
            &command_id(),
        )
        .await,
    );
    s.record(
        "checkpoint_empty_next_step",
        svc.checkpoint_coordinator(
            SLUG,
            Session::checkpoint(|c| c.next_step = "\n".into()),
            "worker",
            &command_id(),
        )
        .await,
    );
    let wake_at = chrono::Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap()
        + chrono::Duration::microseconds(123_456);
    s.record(
        "checkpoint",
        svc.checkpoint_coordinator(
            SLUG,
            Session::checkpoint(|c| {
                c.acknowledge_message_ids = vec![message.id.clone(), message.id.clone()];
                c.wake_at = Some(wake_at);
            }),
            "worker",
            &command_id(),
        )
        .await,
    );
    s.record(
        "checkpoint_stale",
        svc.checkpoint_coordinator(SLUG, Session::checkpoint(|_| {}), "worker", &command_id())
            .await,
    );
    let second = svc
        .send_message(SLUG, "ANSWER", "Use the March snapshot.", None, "owner", &command_id())
        .await
        .unwrap();
    s.record(
        "acknowledge_message",
        svc.acknowledge_message(SLUG, &second.id, "worker", &command_id()).await,
    );
    s.record(
        "acknowledge_missing",
        svc.acknowledge_message(SLUG, "missing", "worker", &command_id()).await,
    );
    s.record("release_coordinator", svc.release_coordinator(SLUG, "worker", &command_id()).await);
    s.record(
        "release_coordinator_again",
        svc.release_coordinator(SLUG, "worker", &command_id()).await,
    );
    s.record(
        "set_variable",
        svc.set_variable(
            SLUG,
            " source_url ",
            " https://example.invalid/data ",
            "owner",
            &command_id(),
        )
        .await,
    );
    s.record(
        "set_secret",
        svc.set_secret(SLUG, "SOURCE_TOKEN", "never-shown", "owner", &command_id()).await,
    );
    s.record(
        "set_variable_reserved",
        svc.set_variable(SLUG, "openai_base_url", "x", "owner", &command_id()).await,
    );
    s.record(
        "set_variable_invalid_name",
        svc.set_variable(SLUG, "1BAD", "x", "owner", &command_id()).await,
    );
    s.record(
        "set_variable_empty",
        svc.set_variable(SLUG, "EMPTY", "  ", "owner", &command_id()).await,
    );
    s.record(
        "set_secret_too_large",
        svc.set_secret(SLUG, "LARGE", &"x".repeat(32_769), "owner", &command_id()).await,
    );
    s.record("list_resources", svc.list_resources(SLUG, true).await);
    s.record("resource_environment", svc.resource_environment(SLUG).await);
    s.record(
        "remove_resource",
        svc.remove_resource(SLUG, "source_token", "owner", &command_id()).await,
    );
    s.record(
        "remove_resource_missing",
        svc.remove_resource(SLUG, "source_token", "owner", &command_id()).await,
    );
    s.record("list_resources_all", svc.list_resources(SLUG, false).await);
    s.record(
        "create_hypothesis",
        svc.create_hypothesis(SLUG, Session::hypothesis(|_| {}), "researcher", &command_id()).await,
    );
    s.record(
        "create_hypothesis_empty_title",
        svc.create_hypothesis(
            SLUG,
            Session::hypothesis(|h| h.title = "\t".into()),
            "researcher",
            &command_id(),
        )
        .await,
    );
    s.record(
        "create_experiment_missing_hypothesis",
        svc.create_experiment(
            SLUG,
            Session::experiment(|e| e.hypothesis_id = "h999".into()),
            "researcher",
            &command_id(),
        )
        .await,
    );
    s.record(
        "create_experiment",
        svc.create_experiment(SLUG, Session::experiment(|_| {}), "researcher", &command_id()).await,
    );
    s.record("get_hypothesis_testing", svc.get_artifact(SLUG, "h001").await);
    s.record(
        "claim_experiment",
        svc.claim_experiment(SLUG, "e001", 600, "worker", &command_id()).await,
    );
    s.record(
        "claim_experiment_taken",
        svc.claim_experiment(SLUG, "E001", 600, "other", &command_id()).await,
    );
    s.record(
        "claim_hypothesis_as_experiment",
        svc.claim_experiment(SLUG, "H001", 600, "worker", &command_id()).await,
    );
    s.record(
        "append_observation",
        svc.append_observation(
            SLUG,
            "e001",
            " NDCG +12%. ",
            Some(" s3://evidence/run.json "),
            "worker",
            &command_id(),
        )
        .await,
    );
    s.record(
        "append_observation_blank_ref",
        svc.append_observation(SLUG, "E001", "Latency +8 ms.", Some(""), "worker", &command_id())
            .await,
    );
    s.record(
        "append_observation_other_actor",
        svc.append_observation(SLUG, "E001", "Not mine.", None, "other", &command_id()).await,
    );
    s.record("get_experiment_running", svc.get_artifact(SLUG, "e001").await);
    s.record(
        "publish_finding_early",
        svc.publish_finding(SLUG, Session::finding(|_| {}), "worker", &command_id()).await,
    );
    let outcome =
        |results: &str, analysis: &str, decision: &str, expected_version| ExperimentOutcome {
            results: results.into(),
            analysis: analysis.into(),
            decision: decision.into(),
            expected_version,
        };
    s.record(
        "complete_experiment_stale",
        svc.complete_experiment(SLUG, "E001", outcome("r", "a", "d", 1), "worker", &command_id())
            .await,
    );
    s.record(
        "complete_experiment",
        svc.complete_experiment(
            SLUG,
            "E001",
            outcome(" Treatment won. ", " Threshold exceeded. ", " Publish. ", 2),
            "worker",
            &command_id(),
        )
        .await,
    );
    s.record("get_experiment_completed", svc.get_artifact(SLUG, "E001").await);
    s.record(
        "publish_finding_bad_impact",
        svc.publish_finding(
            SLUG,
            Session::finding(|f| f.impact = "huge".into()),
            "worker",
            &command_id(),
        )
        .await,
    );
    s.record(
        "publish_finding",
        svc.publish_finding(SLUG, Session::finding(|_| {}), "worker", &command_id()).await,
    );
    let decision = |status: &str, conclusion: &str, expected_version| HypothesisDecision {
        status: status.into(),
        conclusion: conclusion.into(),
        expected_version,
    };
    s.record(
        "decide_hypothesis_bad_status",
        svc.decide_hypothesis(SLUG, "H001", decision("maybe", "x", 2), "reviewer", &command_id())
            .await,
    );
    s.record(
        "decide_hypothesis_stale",
        svc.decide_hypothesis(
            SLUG,
            "h001",
            decision("confirmed", "x", 1),
            "reviewer",
            &command_id(),
        )
        .await,
    );
    s.record(
        "decide_hypothesis",
        svc.decide_hypothesis(
            SLUG,
            "h001",
            decision("confirmed", " Confirmed offline. ", 2),
            "reviewer",
            &command_id(),
        )
        .await,
    );
    s.record(
        "create_experiment_on_confirmed",
        svc.create_experiment(SLUG, Session::experiment(|_| {}), "researcher", &command_id()).await,
    );
    s.record("get_artifact_bad_pattern", svc.get_artifact(SLUG, "nope").await);
    s.record("get_artifact_missing", svc.get_artifact(SLUG, "f999").await);
    s.record("get_artifact_missing_project", svc.get_artifact("missing", "H001").await);
    s.record("list_artifacts", svc.list_artifacts(SLUG, None).await);
    s.record("list_experiments", svc.list_artifacts(SLUG, Some("e")).await);
    s.record("status", svc.status(SLUG).await);
    let payload = json!({
        "kind": "message",
        "text": "Planning.",
        "nested": {"z": 1, "a": [1, 2.5, null, true, "ü"]},
    });
    s.record(
        "record_runtime_event",
        svc.record_runtime_event(
            SLUG,
            "runtime.codex",
            payload.as_object().cloned().unwrap(),
            "limina:worker",
            &command_id(),
        )
        .await,
    );
    s.record(
        "record_runtime_event_bad_type",
        svc.record_runtime_event(SLUG, "coordinator.claimed", Map::new(), "x", &command_id()).await,
    );
    s.record("events", svc.events(SLUG, 0, 200).await);
    s.record("events_page", svc.events(SLUG, 2, 3).await);
    s.record("events_clamped", svc.events(SLUG, 0, 0).await);
    s.record("events_before", svc.events_before(SLUG, 10, 0, 3, HIDDEN).await);
    s.record("events_before_bounded", svc.events_before(SLUG, 10, 6, 100, &[]).await);
    s.record("events_before_empty", svc.events_before(SLUG, 1, 0, 5, &[]).await);
    s.record("has_events_before", svc.has_events_before(SLUG, 3, HIDDEN).await);
    s.record("has_events_before_first", svc.has_events_before(SLUG, 1, &[]).await);
    s.record("recent_events", svc.recent_events(SLUG, 4).await);
    s.change("change_bad_action", "restart").await;
    s.change("change_start_running", "START").await;
    s.change("change_archive_running", "archive").await;
    s.change("change_pause", "pause").await;
    s.change("change_pause_again", "pause").await;
    s.change("change_start_paused", "start").await;
    s.change("change_resume", "resume").await;
    s.change("change_stop", "stop").await;
    s.change("change_start", "start").await;
    s.change("change_stop_again", "stop").await;
    s.change("change_archive", "archive").await;
    s.change("change_archive_again", "archive").await;
    s.change("change_resume_archived", "resume").await;
    s.record("lifecycle_events", svc.recent_events(SLUG, 6).await);
    s.record("list_projects_active", svc.list_projects(false).await);
    s.record("list_projects_all", svc.list_projects(true).await);
    s.results
}

#[tokio::test]
async fn results_and_errors_match_the_python_runtime() {
    let expected: Map<String, Value> =
        serde_json::from_str(include_str!("fixtures/shapes.json")).unwrap();
    each_backend(|backend| {
        let expected = expected.clone();
        async move {
            let actual = run(backend.service()).await;
            let names = |map: &Map<String, Value>| map.keys().cloned().collect::<Vec<_>>();
            assert_eq!(names(&actual), names(&expected), "{}: the same session ran", backend.name);
            for (name, expected) in &expected {
                assert_eq!(
                    serde_json::to_string_pretty(&actual[name]).unwrap(),
                    serde_json::to_string_pretty(expected).unwrap(),
                    "{}: `{name}` differs from the Python runtime",
                    backend.name
                );
            }
        }
    })
    .await;
}
