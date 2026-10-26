//! Idempotent replay and concurrent commands: a command ID always yields its first result
//! (including receipts the Python runtime stored), and concurrent writers never corrupt state.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::too_many_lines)]

mod service_support;

use limina_core::{Result, now};
use limina_domain::project::NewChallenge;
use limina_domain::research::{
    ExperimentOutcome, HypothesisDecision, NewExperiment, NewFinding, NewHypothesis,
};
use limina_domain::service::{ChallengeService, Checkpoint};
use limina_store::{Db, args};
use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value};
use service_support::{
    code, command_id, create_experiment, create_hypothesis, each_backend, hypothesis, retrieval,
};

async fn event_count(db: &Db) -> i64 {
    let mut tx = db.begin().await.unwrap();
    let count = tx.scalar("SELECT CAST(COUNT(*) AS BIGINT) FROM events", args![]).await.unwrap();
    tx.commit().await.unwrap();
    count
}

fn json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap()
}

/// Runs a command twice with the same command ID: the second run returns the stored result and
/// writes nothing.
macro_rules! twice {
    ($db:expr, $call:expr) => {{
        let before = event_count($db).await;
        let first = $call.await.unwrap();
        let after = event_count($db).await;
        assert_eq!(after, before + 1, "the command records one event");
        let again = $call.await.unwrap();
        assert_eq!(json(&first), json(&again), "a replay returns the first result");
        assert_eq!(event_count($db).await, after, "a replay writes nothing");
        first
    }};
}

#[tokio::test]
async fn every_command_replays_its_first_result() {
    each_backend(|backend| async move {
        let service = backend.service();
        let db = &backend.db;
        let id = command_id();
        let created = twice!(
            db,
            service.create_challenge(
                NewChallenge {
                    slug: "retrieval".into(),
                    name: "Retrieval".into(),
                    objective: "Improve retrieval.".into(),
                    success_criteria: "NDCG.".into(),
                    ..NewChallenge::default()
                },
                "owner",
                &id,
            )
        );
        assert_eq!(created.slug, "retrieval");
        let id = command_id();
        twice!(db, service.claim_coordinator("retrieval", 300, "worker", &id));
        let id = command_id();
        let message =
            twice!(db, service.send_message("retrieval", "STEER", "Go.", None, "owner", &id));
        let id = command_id();
        twice!(
            db,
            service.checkpoint_coordinator(
                "retrieval",
                Checkpoint {
                    current_objective: "Objective.".into(),
                    next_step: "Next.".into(),
                    status: "RUNNING".into(),
                    expected_version: 1,
                    acknowledge_message_ids: vec![message.id.clone()],
                    ..Checkpoint::default()
                },
                "worker",
                &id,
            )
        );
        let id = command_id();
        twice!(db, service.acknowledge_message("retrieval", &message.id, "worker", &id));
        let id = command_id();
        twice!(db, service.release_coordinator("retrieval", "worker", &id));
        let id = command_id();
        twice!(db, service.set_variable("retrieval", "SOURCE_URL", "s3://data", "owner", &id));
        let id = command_id();
        twice!(db, service.set_secret("retrieval", "SOURCE_TOKEN", "secret", "owner", &id));
        let id = command_id();
        twice!(db, service.remove_resource("retrieval", "SOURCE_TOKEN", "owner", &id));
        let id = command_id();
        let h = twice!(
            db,
            service.create_hypothesis("retrieval", hypothesis("Reranking"), "researcher", &id)
        );
        let id = command_id();
        let e = twice!(
            db,
            service.create_experiment(
                "retrieval",
                service_support::experiment(&h.id, "Benchmark"),
                "researcher",
                &id,
            )
        );
        let claim_id = command_id();
        let claim =
            twice!(db, service.claim_experiment("retrieval", &e.id, 300, "worker", &claim_id));
        let id = command_id();
        twice!(db, service.append_observation("retrieval", &e.id, "Seen.", None, "worker", &id));
        let id = command_id();
        let outcome = ExperimentOutcome {
            results: "R.".into(),
            analysis: "A.".into(),
            decision: "D.".into(),
            expected_version: claim.artifact.version,
        };
        twice!(db, service.complete_experiment("retrieval", &e.id, outcome.clone(), "worker", &id));
        let id = command_id();
        let finding = NewFinding {
            experiment_id: e.id.clone(),
            title: "F".into(),
            finding: "F.".into(),
            evidence: "E.".into(),
            impact: "LOW".into(),
            ..NewFinding::default()
        };
        twice!(db, service.publish_finding("retrieval", finding.clone(), "worker", &id));
        let id = command_id();
        let decision = HypothesisDecision {
            status: "CONFIRMED".into(),
            conclusion: "Yes.".into(),
            expected_version: 2,
        };
        twice!(
            db,
            service.decide_hypothesis("retrieval", &h.id, decision.clone(), "reviewer", &id)
        );
        let id = command_id();
        twice!(
            db,
            service.record_runtime_event(
                "retrieval",
                "runtime.codex",
                Map::new(),
                "limina:worker",
                &id
            )
        );
        let id = command_id();
        twice!(db, service.change_project_state("retrieval", "pause", "owner", &id));

        // A replay returns the stored result even after the state moved on.
        let replayed = service.claim_experiment("retrieval", &e.id, 300, "worker", &claim_id).await;
        assert_eq!(json(&replayed.unwrap()), json(&claim));
        assert_eq!(
            service.get_artifact("retrieval", &e.id).await.unwrap().artifact.status,
            "COMPLETED"
        );
    })
    .await;
}

#[tokio::test]
async fn command_ids_belong_to_one_actor_and_one_command_type() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let id = command_id();
        service
            .create_hypothesis("retrieval", hypothesis("Reranking"), "researcher", &id)
            .await
            .unwrap();
        let other_actor =
            service.create_hypothesis("retrieval", hypothesis("Reranking"), "intruder", &id).await;
        let error = other_actor.unwrap_err();
        assert_eq!(error.code(), "version_conflict");
        assert_eq!(error.message, format!("Command ID '{id}' belongs to another actor."));
        let other_type = service
            .send_message("retrieval", "STEER", "Hello.", None, "researcher", &id)
            .await
            .unwrap_err();
        assert_eq!(other_type.code(), "version_conflict");
        assert_eq!(
            other_type.message,
            format!("Command ID '{id}' was already used for hypothesis.create.")
        );
        assert_eq!(other_type.details["original_command_type"], "hypothesis.create");
        assert_eq!(
            code(service.claim_coordinator("retrieval", 300, " ", &command_id()).await),
            "invariant_violation"
        );
        assert_eq!(
            code(service.claim_coordinator("retrieval", 300, "worker", "  ").await),
            "invariant_violation"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_retries_of_one_command_write_once() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let id = command_id();
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let service = service.clone();
                let id = id.clone();
                tokio::spawn(async move {
                    service
                        .create_hypothesis("retrieval", hypothesis("Once"), "researcher", &id)
                        .await
                })
            })
            .collect();
        let mut results = Vec::new();
        for task in tasks {
            results.push(json(&task.await.unwrap().unwrap()));
        }
        results.dedup();
        assert_eq!(results.len(), 1, "every retry returns the winner's result");
        let artifacts = service.list_artifacts("retrieval", Some("H")).await.unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].id, "H001");
    })
    .await;
}

/// Runs the same kind of write from several tasks at once and returns each outcome.
async fn race<T, F, Fut>(count: usize, call: F) -> Vec<Result<T>>
where
    T: Send + 'static,
    F: Fn(usize) -> Fut,
    Fut: std::future::Future<Output = Result<T>> + Send + 'static,
{
    let tasks: Vec<_> = (0..count).map(|index| tokio::spawn(call(index))).collect();
    let mut outcomes = Vec::new();
    for task in tasks {
        outcomes.push(task.await.unwrap());
    }
    outcomes
}

fn winners<T>(outcomes: &[Result<T>], losing_code: &str) -> usize {
    for outcome in outcomes {
        if let Err(error) = outcome {
            assert_eq!(error.code(), losing_code, "{}", error.message);
        }
    }
    outcomes.iter().filter(|outcome| outcome.is_ok()).count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_conflicting_commands_have_one_winner() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;

        let s = service.clone();
        let created = race(4, move |index| {
            let s = s.clone();
            async move {
                s.create_challenge(
                    NewChallenge {
                        slug: "contested".into(),
                        name: format!("Contested {index}"),
                        objective: "O.".into(),
                        success_criteria: "S.".into(),
                        ..NewChallenge::default()
                    },
                    "owner",
                    &command_id(),
                )
                .await
            }
        })
        .await;
        assert_eq!(winners(&created, "version_conflict"), 1);

        let s = service.clone();
        let claims = race(6, move |index| {
            let s = s.clone();
            async move {
                s.claim_coordinator("retrieval", 300, &format!("worker-{index}"), &command_id())
                    .await
            }
        })
        .await;
        assert_eq!(winners(&claims, "lease_conflict"), 1);

        let s = service.clone();
        let checkpoints = race(6, move |index| {
            let s = s.clone();
            async move {
                s.checkpoint_coordinator(
                    "retrieval",
                    Checkpoint {
                        current_objective: format!("Objective {index}."),
                        next_step: "Next.".into(),
                        status: "RUNNING".into(),
                        expected_version: 1,
                        ..Checkpoint::default()
                    },
                    "worker",
                    &command_id(),
                )
                .await
            }
        })
        .await;
        assert_eq!(winners(&checkpoints, "version_conflict"), 1);
        assert_eq!(
            service.get_challenge("retrieval").await.unwrap().coordinator.unwrap().version,
            2
        );

        let h = create_hypothesis(&service, "Contested").await;
        let s = service.clone();
        let id = h.id.clone();
        let decisions = race(6, move |index| {
            let (s, id) = (s.clone(), id.clone());
            async move {
                s.decide_hypothesis(
                    "retrieval",
                    &id,
                    HypothesisDecision {
                        status: if index % 2 == 0 { "CONFIRMED" } else { "REJECTED" }.into(),
                        conclusion: format!("Decision {index}."),
                        expected_version: 1,
                    },
                    "reviewer",
                    &command_id(),
                )
                .await
            }
        })
        .await;
        assert_eq!(winners(&decisions, "version_conflict"), 1);
        assert_eq!(service.get_artifact("retrieval", &h.id).await.unwrap().artifact.version, 2);

        let s = service.clone();
        let variables = race(4, move |index| {
            let s = s.clone();
            async move {
                s.set_variable(
                    "retrieval",
                    "SHARED",
                    &format!("value-{index}"),
                    "owner",
                    &command_id(),
                )
                .await
            }
        })
        .await;
        // SQLite serializes writers (later writes update the row); PostgreSQL may reject a
        // concurrent first insert as a conflict. Either way one row remains.
        assert!(winners(&variables, "version_conflict") >= 1);
        let shared = service
            .list_resources("retrieval", false)
            .await
            .unwrap()
            .into_iter()
            .filter(|item| item.name == "SHARED")
            .count();
        assert_eq!(shared, 1);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn experiment_claims_race_to_one_owner() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let h = create_hypothesis(&service, "Reranking").await;
        let e = create_experiment(&service, &h.id, "Benchmark").await;
        let s = service.clone();
        let id = e.id.clone();
        let claims = race(6, move |index| {
            let (s, id) = (s.clone(), id.clone());
            async move {
                s.claim_experiment("retrieval", &id, 300, &format!("worker-{index}"), &command_id())
                    .await
            }
        })
        .await;
        assert_eq!(winners(&claims, "lease_conflict"), 1);
        let detail = service.get_artifact("retrieval", &e.id).await.unwrap();
        assert_eq!(detail.artifact.status, "RUNNING");
        assert_eq!(detail.artifact.version, 2);
    })
    .await;
}

/// The receipts the Python runtime stored (`fixtures/receipts.json`) replay through the Rust
/// commands and return the same JSON. The only difference: lease expiry times that the Python
/// runtime returned in SQLite's raw `YYYY-MM-DD HH:MM:SS+00:00` text come back as ISO-8601 with
/// a `T`, as they already were on PostgreSQL.
#[tokio::test]
async fn receipts_written_by_the_python_runtime_replay() {
    let receipts: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/receipts.json")).unwrap();
    each_backend(|backend| {
        let receipts = receipts.clone();
        async move {
            let service = backend.service();
            let mut tx = backend.db.begin_write().await.unwrap();
            for receipt in &receipts {
                let result: Value =
                    serde_json::from_str(receipt["result"].as_str().unwrap()).unwrap();
                tx.execute(
                    "INSERT INTO command_receipts (command_id, command_type, actor, result, \
                     created_at) VALUES ($1, $2, $3, $4, $5)",
                    args![
                        receipt["command_id"].as_str(),
                        receipt["command_type"].as_str(),
                        receipt["actor"].as_str(),
                        result,
                        now()
                    ],
                )
                .await
                .unwrap();
            }
            tx.commit().await.unwrap();

            let space =
                Regex::new(r"^(\d{4}-\d{2}-\d{2}) (\d{2}:\d{2}:\d{2}(\.\d+)?\+00:00)$").unwrap();
            let mut python_sqlite_times = 0;
            let mut kinds = std::collections::BTreeSet::new();
            for receipt in &receipts {
                let command_type = receipt["command_type"].as_str().unwrap();
                let original: Value =
                    serde_json::from_str(receipt["result"].as_str().unwrap()).unwrap();
                let expected = iso_times(original, &space, &mut python_sqlite_times);
                let replayed = replay(
                    &service,
                    command_type,
                    receipt["command_id"].as_str().unwrap(),
                    receipt["actor"].as_str().unwrap(),
                )
                .await
                .unwrap_or_else(|error| panic!("{command_type}: {}", error.message));
                assert_eq!(json(&replayed), json(&expected), "{}: {command_type}", backend.name);
                kinds.insert(command_type.to_owned());
            }
            assert!(python_sqlite_times > 0, "the fixture covers Python's SQLite lease times");
            assert!(kinds.len() >= 20, "the fixture covers every command type: {kinds:?}");
            assert_eq!(event_count(&backend.db).await, 0, "replays write nothing");
        }
    })
    .await;
}

/// Rewrites `YYYY-MM-DD HH:MM:SS+00:00` strings as ISO-8601 with a `T` separator.
fn iso_times(value: Value, space: &Regex, count: &mut usize) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter().map(|(key, value)| (key, iso_times(value, space, count))).collect(),
        ),
        Value::Array(items) => {
            Value::Array(items.into_iter().map(|item| iso_times(item, space, count)).collect())
        }
        Value::String(text) if space.is_match(&text) => {
            *count += 1;
            Value::String(space.replace(&text, "${1}T${2}").into_owned())
        }
        other => other,
    }
}

/// Calls the command of `command_type`. Inputs only need to be valid: a stored receipt is
/// returned before the command reads any state.
async fn replay(
    service: &ChallengeService,
    command_type: &str,
    id: &str,
    actor: &str,
) -> Result<Value> {
    fn value<T: Serialize>(result: Result<T>) -> Result<Value> {
        result.map(|value| serde_json::to_value(value).unwrap())
    }
    let slug = "anything";
    match command_type {
        "challenge.create" => value(
            service
                .create_challenge(
                    NewChallenge {
                        slug: slug.into(),
                        name: "x".into(),
                        objective: "x".into(),
                        success_criteria: "x".into(),
                        ..NewChallenge::default()
                    },
                    actor,
                    id,
                )
                .await,
        ),
        "coordinator.claim" => value(service.claim_coordinator(slug, 300, actor, id).await),
        "coordinator.release" => value(service.release_coordinator(slug, actor, id).await),
        "coordinator.checkpoint" => value(
            service
                .checkpoint_coordinator(
                    slug,
                    Checkpoint {
                        current_objective: "x".into(),
                        next_step: "x".into(),
                        status: "RUNNING".into(),
                        ..Checkpoint::default()
                    },
                    actor,
                    id,
                )
                .await,
        ),
        "inbox.send" => value(service.send_message(slug, "STEER", "x", None, actor, id).await),
        "inbox.acknowledge" => value(service.acknowledge_message(slug, "x", actor, id).await),
        "resource.variable.set" => value(service.set_variable(slug, "X", "x", actor, id).await),
        "resource.secret.set" => value(service.set_secret(slug, "X", "x", actor, id).await),
        "resource.remove" => value(service.remove_resource(slug, "X", actor, id).await),
        "hypothesis.create" => value(
            service
                .create_hypothesis(
                    slug,
                    NewHypothesis {
                        title: "x".into(),
                        statement: "x".into(),
                        ..NewHypothesis::default()
                    },
                    actor,
                    id,
                )
                .await,
        ),
        "hypothesis.decide" => value(
            service
                .decide_hypothesis(
                    slug,
                    "H001",
                    HypothesisDecision {
                        status: "CONFIRMED".into(),
                        conclusion: "x".into(),
                        expected_version: 1,
                    },
                    actor,
                    id,
                )
                .await,
        ),
        "experiment.create" => value(
            service
                .create_experiment(
                    slug,
                    NewExperiment {
                        hypothesis_id: "H001".into(),
                        title: "x".into(),
                        objective: "x".into(),
                        ..NewExperiment::default()
                    },
                    actor,
                    id,
                )
                .await,
        ),
        "experiment.claim" => value(service.claim_experiment(slug, "E001", 300, actor, id).await),
        "experiment.observe" => {
            value(service.append_observation(slug, "E001", "x", None, actor, id).await)
        }
        "experiment.complete" => value(
            service
                .complete_experiment(
                    slug,
                    "E001",
                    ExperimentOutcome {
                        results: "x".into(),
                        analysis: "x".into(),
                        decision: "x".into(),
                        expected_version: 1,
                    },
                    actor,
                    id,
                )
                .await,
        ),
        "finding.publish" => value(
            service
                .publish_finding(
                    slug,
                    NewFinding {
                        experiment_id: "E001".into(),
                        title: "x".into(),
                        finding: "x".into(),
                        evidence: "x".into(),
                        impact: "LOW".into(),
                        ..NewFinding::default()
                    },
                    actor,
                    id,
                )
                .await,
        ),
        action if action.starts_with("project.") => value(
            service
                .change_project_state(slug, action.trim_start_matches("project."), actor, id)
                .await,
        ),
        event_type if event_type.starts_with("runtime.") => {
            value(service.record_runtime_event(slug, event_type, Map::new(), actor, id).await)
        }
        other => panic!("no command for {other}"),
    }
}
