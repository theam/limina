//! The Markdown export is byte-identical to the Python exporter. `fixtures/export/expected/` is
//! what the Python runtime exported for `fixtures/export/dataset.json` (a frozen recording).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod service_support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use limina_core::time::parse;
use limina_domain::exporter::MarkdownExporter;
use limina_domain::research::{ExperimentOutcome, NewFinding};
use limina_store::{Arg, Db};
use serde_json::Value;
use service_support::{command_id, create_experiment, create_hypothesis, each_backend, retrieval};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/export")
}

fn time(value: &Value) -> Arg {
    Arg::from(value.as_str().map(|text| parse(text).unwrap()))
}

fn text(value: &Value) -> Arg {
    Arg::from(value.as_str())
}

/// Loads the dataset's rows exactly as stored, bypassing the commands.
async fn load_dataset(db: &Db) -> String {
    let dataset: Value =
        serde_json::from_str(&std::fs::read_to_string(fixtures().join("dataset.json")).unwrap())
            .unwrap();
    let challenge = &dataset["challenge"];
    let mut tx = db.begin_write().await.unwrap();
    tx.execute(
        "INSERT INTO challenges (id, slug, name, objective, context, success_criteria, \
         runtime_engine, status, version, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        vec![
            text(&challenge["id"]),
            text(&challenge["slug"]),
            text(&challenge["name"]),
            text(&challenge["objective"]),
            text(&challenge["context"]),
            text(&challenge["success_criteria"]),
            text(&challenge["runtime_engine"]),
            text(&challenge["status"]),
            Arg::from(i32::try_from(challenge["version"].as_i64().unwrap()).unwrap()),
            time(&challenge["created_at"]),
            time(&challenge["updated_at"]),
        ],
    )
    .await
    .unwrap();
    let coordinator = &dataset["coordinator"];
    tx.execute(
        "INSERT INTO coordinator_states (challenge_id, status, current_objective, next_step, \
         blocker, worker_id, continuation_id, inbox_cursor, version, heartbeat_at, wake_at, \
         updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        vec![
            text(&challenge["id"]),
            text(&coordinator["status"]),
            text(&coordinator["current_objective"]),
            text(&coordinator["next_step"]),
            text(&coordinator["blocker"]),
            text(&coordinator["worker_id"]),
            text(&coordinator["continuation_id"]),
            Arg::from(i32::try_from(coordinator["inbox_cursor"].as_i64().unwrap()).unwrap()),
            Arg::from(i32::try_from(coordinator["version"].as_i64().unwrap()).unwrap()),
            time(&coordinator["heartbeat_at"]),
            time(&coordinator["wake_at"]),
            time(&coordinator["updated_at"]),
        ],
    )
    .await
    .unwrap();
    for item in dataset["artifacts"].as_array().unwrap() {
        tx.execute(
            "INSERT INTO artifacts (uid, challenge_id, artifact_id, kind, title, status, payload, \
             parent_hypothesis_id, parent_experiment_id, version, created_by, created_at, \
             updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
            vec![
                text(&item["uid"]),
                text(&challenge["id"]),
                text(&item["artifact_id"]),
                text(&item["kind"]),
                text(&item["title"]),
                text(&item["status"]),
                Arg::from(item["payload"].clone()),
                text(&item["parent_hypothesis_id"]),
                text(&item["parent_experiment_id"]),
                Arg::from(i32::try_from(item["version"].as_i64().unwrap()).unwrap()),
                text(&item["created_by"]),
                time(&item["created_at"]),
                time(&item["updated_at"]),
            ],
        )
        .await
        .unwrap();
    }
    for item in dataset["observations"].as_array().unwrap() {
        tx.execute(
            "INSERT INTO observations (id, challenge_id, experiment_id, body, evidence_ref, \
             actor, command_id, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            vec![
                text(&item["id"]),
                text(&challenge["id"]),
                text(&item["experiment_id"]),
                text(&item["body"]),
                text(&item["evidence_ref"]),
                text(&item["actor"]),
                text(&item["command_id"]),
                time(&item["created_at"]),
            ],
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    challenge["slug"].as_str().unwrap().to_owned()
}

fn expected_files() -> BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, files: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, files);
            } else {
                let relative =
                    path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                files.insert(relative, std::fs::read_to_string(&path).unwrap());
            }
        }
    }
    let root = fixtures().join("expected");
    let mut files = BTreeMap::new();
    walk(&root, &root, &mut files);
    files
}

#[tokio::test]
async fn snapshot_is_byte_identical_to_the_python_exporter() {
    let expected = expected_files();
    assert_eq!(expected.len(), 11);
    each_backend(|backend| {
        let expected = expected.clone();
        async move {
            let slug = load_dataset(&backend.db).await;
            let snapshot = MarkdownExporter::new(backend.service()).snapshot(&slug).await.unwrap();
            assert_eq!(
                snapshot.keys().collect::<Vec<_>>(),
                expected.keys().collect::<Vec<_>>(),
                "{}: the same files",
                backend.name
            );
            for (path, content) in &expected {
                assert_eq!(&snapshot[path], content, "{}: {path}", backend.name);
            }
        }
    })
    .await;
}

#[tokio::test]
async fn write_fills_an_empty_target_and_refuses_a_used_one() {
    each_backend(|backend| async move {
        let slug = load_dataset(&backend.db).await;
        let exporter = MarkdownExporter::new(backend.service());
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("kb");
        let written = exporter.write(&slug, &target).await.unwrap();
        let expected = expected_files();
        assert_eq!(written.len(), expected.len());
        assert_eq!(written[0], target.join("ACTIVE.md"), "paths come in snapshot order");
        for (relative, content) in &expected {
            assert_eq!(&std::fs::read_to_string(target.join(relative)).unwrap(), content);
        }

        let again = exporter.write(&slug, &target).await.unwrap_err();
        assert_eq!(again.code(), "version_conflict");
        assert_eq!(again.message, format!("Export target '{}' is not empty.", target.display()));
        assert_eq!(again.details["target"], target.display().to_string());
        assert_eq!(
            again.details["suggestion"],
            "Choose an empty directory so stale artifacts cannot survive the export."
        );

        let empty = dir.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        assert_eq!(exporter.write(&slug, &empty).await.unwrap().len(), expected.len());
        let missing = exporter.write("missing", &dir.path().join("other")).await.unwrap_err();
        assert_eq!(missing.code(), "not_found");
    })
    .await;
}

/// The Python suite's `test_full_chain_exports_to_a_validator_compatible_kb`: a full H → E → F
/// chain exports the notes and links `scripts/kb_validate.py` requires.
#[tokio::test]
async fn full_chain_exports_a_linked_kb() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let h = create_hypothesis(&service, "Reranking").await;
        let e = create_experiment(&service, &h.id, "Benchmark").await;
        let claimed = service
            .claim_experiment("retrieval", &e.id, 300, "worker", &command_id())
            .await
            .unwrap();
        service
            .append_observation(
                "retrieval",
                &e.id,
                "NDCG improved by twelve percent.",
                Some("s3://evidence/run.json"),
                "worker",
                &command_id(),
            )
            .await
            .unwrap();
        service
            .complete_experiment(
                "retrieval",
                &e.id,
                ExperimentOutcome {
                    results: "Treatment improved NDCG by twelve percent.".into(),
                    analysis: "The threshold was exceeded under the fixed candidate set.".into(),
                    decision: "Publish a finding and test online latency.".into(),
                    expected_version: claimed.artifact.version,
                },
                "worker",
                &command_id(),
            )
            .await
            .unwrap();
        service
            .publish_finding(
                "retrieval",
                NewFinding {
                    experiment_id: e.id.clone(),
                    title: "Reranking improves offline NDCG".into(),
                    finding: "The treatment exceeded the offline NDCG threshold.".into(),
                    evidence: "A fixed-candidate comparison improved NDCG by twelve percent."
                        .into(),
                    improvement: "Relevance ordering improved on held-out queries.".into(),
                    remaining_debt: "Online latency is not established.".into(),
                    next_move: "Measure latency in a production-like environment.".into(),
                    impact: "HIGH".into(),
                },
                "worker",
                &command_id(),
            )
            .await
            .unwrap();

        let snapshot = MarkdownExporter::new(service).snapshot("retrieval").await.unwrap();
        let paths: Vec<&str> = snapshot.keys().map(String::as_str).collect();
        assert_eq!(
            paths,
            [
                "ACTIVE.md",
                "DASHBOARD.md",
                "mission/CHALLENGE.md",
                "research/experiments/E001-benchmark.md",
                "research/findings/F001-reranking-improves-offline-ndcg.md",
                "research/hypotheses/H001-reranking.md",
            ]
        );
        let today = limina_core::now().format("%Y-%m-%d").to_string();
        let experiment = &snapshot["research/experiments/E001-benchmark.md"];
        assert!(experiment.contains(&format!("completed: \"{today}\"\n")));
        assert!(experiment.contains(&format!(
            "- [x] ({today}) NDCG improved by twelve percent. — `s3://evidence/run.json`\n"
        )));
        assert!(experiment.ends_with("- Parent Hypothesis: [[H001]]\n- Finding: [[F001]]\n"));
        let hypothesis = &snapshot["research/hypotheses/H001-reranking.md"];
        assert!(hypothesis.ends_with("- Experiment: [[E001]]\n- Finding: [[F001]]\n"));
        assert!(
            snapshot["ACTIVE.md"].contains("- Mission: [[CHALLENGE]]\n- Working Note: [[H001]]\n")
        );
        for (path, content) in &snapshot {
            assert!(content.contains("\n## Links\n\n"), "{path} has a Links section");
        }
    })
    .await;
}
