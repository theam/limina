//! Ports of the service-level tests in `tests/test_cloud_runtime.py`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod service_support;

use std::collections::BTreeSet;
use std::sync::Arc;

use limina_domain::project::NewChallenge;
use limina_domain::research::{ExperimentOutcome, HypothesisDecision, NewFinding};
use limina_domain::service::{ChallengeService, Checkpoint};
use limina_store::{SecretCipher, args};
use service_support::{
    code, command_id, create_experiment, create_hypothesis, each_backend, experiment, hypothesis,
    retrieval,
};

#[tokio::test]
async fn sqlite_utc_timestamps_survive_a_read_round_trip() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let created = service
            .create_hypothesis(
                "retrieval",
                hypothesis("Timestamp round trip"),
                "researcher",
                &command_id(),
            )
            .await
            .unwrap();
        let loaded = service.get_artifact("retrieval", &created.id).await.unwrap();
        let created_json = serde_json::to_value(&created).unwrap();
        let loaded_json = serde_json::to_value(&loaded).unwrap();
        assert_eq!(created_json["created_at"], loaded_json["created_at"]);
        assert!(loaded_json["created_at"].as_str().unwrap().ends_with("+00:00"));
    })
    .await;
}

#[tokio::test]
async fn project_runtime_engine_is_explicit_and_immutable() {
    each_backend(|backend| async move {
        let service = backend.service();
        let new = |slug: &str, engine: &str| NewChallenge {
            slug: slug.into(),
            name: "Claude research".into(),
            objective: "Run with Claude Code.".into(),
            success_criteria: "Persist the selected engine.".into(),
            runtime_engine: Some(engine.into()),
            ..NewChallenge::default()
        };
        let created = service
            .create_challenge(new("claude-research", "claude-code"), "owner", &command_id())
            .await
            .unwrap();
        assert_eq!(created.runtime_engine, "claude-code");
        assert_eq!(
            service.get_challenge("claude-research").await.unwrap().runtime_engine,
            "claude-code"
        );
        let rejected =
            service.create_challenge(new("unknown-engine", "other"), "owner", &command_id()).await;
        assert_eq!(code(rejected), "invariant_violation");
        let defaulted = service
            .create_challenge(
                NewChallenge { runtime_engine: None, ..new("default-engine", "") },
                "owner",
                &command_id(),
            )
            .await
            .unwrap();
        assert_eq!(defaulted.runtime_engine, "codex");
    })
    .await;
}

#[tokio::test]
async fn hef_invariants_are_enforced_transactionally() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let missing = service
            .create_experiment(
                "retrieval",
                experiment("H999", "Invalid"),
                "researcher",
                &command_id(),
            )
            .await;
        assert_eq!(code(missing), "not_found");

        let hypothesis = create_hypothesis(&service, "Reranking").await;
        let experiment = create_experiment(&service, &hypothesis.id, "Benchmark").await;
        let premature = service
            .publish_finding(
                "retrieval",
                NewFinding {
                    experiment_id: experiment.id.clone(),
                    title: "Premature".into(),
                    finding: "Not established.".into(),
                    evidence: "None.".into(),
                    impact: "LOW".into(),
                    ..NewFinding::default()
                },
                "researcher",
                &command_id(),
            )
            .await;
        assert_eq!(code(premature), "invariant_violation");
        let findings = service.list_artifacts("retrieval", Some("F")).await.unwrap();
        assert!(findings.is_empty(), "a rejected command writes nothing");
    })
    .await;
}

#[tokio::test]
async fn command_replay_is_idempotent() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let replay_key = command_id();
        let first = service
            .create_hypothesis("retrieval", hypothesis("Stable replay"), "researcher", &replay_key)
            .await
            .unwrap();
        let second = service
            .create_hypothesis("retrieval", hypothesis("Stable replay"), "researcher", &replay_key)
            .await
            .unwrap();
        assert_eq!(serde_json::to_string(&first).unwrap(), serde_json::to_string(&second).unwrap());
        assert_eq!(service.list_artifacts("retrieval", Some("H")).await.unwrap().len(), 1);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn parallel_artifact_creation_allocates_unique_ids() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let tasks: Vec<_> = (0..12)
            .map(|index| {
                let service = service.clone();
                tokio::spawn(async move {
                    let mut new = hypothesis(&format!("Hypothesis {index}"));
                    new.statement = format!("Claim {index}");
                    service
                        .create_hypothesis(
                            "retrieval",
                            new,
                            &format!("worker-{index}"),
                            &command_id(),
                        )
                        .await
                        .unwrap()
                        .id
                })
            })
            .collect();
        let mut ids = Vec::new();
        for task in tasks {
            ids.push(task.await.unwrap());
        }
        let unique: BTreeSet<String> = ids.iter().cloned().collect();
        assert_eq!(unique.len(), ids.len());
        let expected: BTreeSet<String> = (1..=12).map(|index| format!("H{index:03}")).collect();
        assert_eq!(unique, expected);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn parallel_experiments_share_only_the_hypothesis_transition() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let hypothesis = create_hypothesis(&service, "Reranking").await;
        let tasks: Vec<_> = (0..12)
            .map(|index| {
                let service = service.clone();
                let hypothesis_id = hypothesis.id.clone();
                tokio::spawn(async move {
                    create_experiment(
                        &service,
                        &hypothesis_id,
                        &format!("Independent lane {index}"),
                    )
                    .await
                    .id
                })
            })
            .collect();
        let mut ids = BTreeSet::new();
        for task in tasks {
            ids.insert(task.await.unwrap());
        }
        let expected: BTreeSet<String> = (1..=12).map(|index| format!("E{index:03}")).collect();
        assert_eq!(ids, expected);
        let updated = service.get_artifact("retrieval", &hypothesis.id).await.unwrap().artifact;
        assert_eq!(updated.status, "TESTING");
        assert_eq!(updated.version, 2);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn experiment_leases_are_scoped_not_global() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let hypothesis = create_hypothesis(&service, "Reranking").await;
        let first = create_experiment(&service, &hypothesis.id, "First lane").await;
        let second = create_experiment(&service, &hypothesis.id, "Second lane").await;
        let claim = |artifact_id: String, actor: &'static str| {
            let service = service.clone();
            tokio::spawn(async move {
                service.claim_experiment("retrieval", &artifact_id, 300, actor, &command_id()).await
            })
        };
        let a = claim(first.id.clone(), "worker-a");
        let b = claim(second.id.clone(), "worker-b");
        let owners: BTreeSet<String> =
            [a.await.unwrap().unwrap(), b.await.unwrap().unwrap()].map(|c| c.lease.owner).into();
        assert_eq!(owners, BTreeSet::from(["worker-a".to_owned(), "worker-b".to_owned()]));
        let stolen = claim(first.id.clone(), "worker-b").await.unwrap();
        assert_eq!(code(stolen), "lease_conflict");
    })
    .await;
}

#[tokio::test]
async fn coordinator_lease_prevents_duplicate_execution() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        service.claim_coordinator("retrieval", 300, "worker-a", &command_id()).await.unwrap();
        let taken = service.claim_coordinator("retrieval", 300, "worker-b", &command_id()).await;
        assert_eq!(code(taken), "lease_conflict");
        service.release_coordinator("retrieval", "worker-a", &command_id()).await.unwrap();
        let claimed =
            service.claim_coordinator("retrieval", 300, "worker-b", &command_id()).await.unwrap();
        assert_eq!(claimed.owner, "worker-b");
        assert_eq!(claimed.scope, "coordinator");
        let renewed =
            service.claim_coordinator("retrieval", 600, "worker-b", &command_id()).await.unwrap();
        assert_eq!(renewed.version, claimed.version + 1, "the owner renews its own lease");
        assert_ne!(renewed.token, claimed.token);
    })
    .await;
}

#[tokio::test]
async fn expired_leases_can_be_taken_over() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        service.claim_coordinator("retrieval", 300, "worker-a", &command_id()).await.unwrap();
        let mut tx = backend.db.begin_write().await.unwrap();
        tx.execute(
            "UPDATE work_leases SET expires_at = $1",
            args![limina_core::now() - chrono::Duration::seconds(1)],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let taken =
            service.claim_coordinator("retrieval", 300, "worker-b", &command_id()).await.unwrap();
        assert_eq!(taken.owner, "worker-b");
        assert_eq!(taken.version, 2);
    })
    .await;
}

#[tokio::test]
async fn secret_resources_are_encrypted_redacted_and_project_bound() {
    each_backend(|backend| async move {
        let cipher = Arc::new(SecretCipher::ephemeral());
        let service = ChallengeService::new(backend.db.clone(), Some(cipher.clone()));
        retrieval(&service).await;
        let plaintext = "never-store-this-plaintext";
        let resource = service
            .set_secret("retrieval", "SERVICE_TOKEN", plaintext, "owner", &command_id())
            .await
            .unwrap();
        assert_eq!(resource.resource_type, "SECRET");
        assert_eq!(resource.value, None);
        assert_eq!(resource.configured, Some(true));

        let mut tx = backend.db.begin().await.unwrap();
        let ciphertext: String = tx
            .scalar(
                "SELECT secret_ciphertext FROM project_resources WHERE name = 'SERVICE_TOKEN'",
                args![],
            )
            .await
            .unwrap();
        let receipts: Vec<(String,)> = tx
            .fetch_all("SELECT CAST(result AS TEXT) FROM command_receipts", args![])
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert!(!ciphertext.contains(plaintext));
        assert!(!format!("{receipts:?}").contains(plaintext));
        let listed = service.list_resources("retrieval", true).await.unwrap();
        assert!(!serde_json::to_string(&listed).unwrap().contains(plaintext));
        let events = service.events("retrieval", 0, 200).await.unwrap();
        assert!(!serde_json::to_string(&events).unwrap().contains(plaintext));
        assert_eq!(
            service.resource_environment("retrieval").await.unwrap()["SERVICE_TOKEN"],
            plaintext
        );
        assert_eq!(
            cipher.decrypt("other-project", "SERVICE_TOKEN", &ciphertext).unwrap_err().code(),
            "invariant_violation"
        );

        for reserved in [
            "PATH",
            "LIMINA_INTERNAL_TOKEN",
            "OPENAI_BASE_URL",
            "ANTHROPIC_BASE_URL",
            "CLAUDE_CONFIG_DIR",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "NODE_OPTIONS",
            "BASH_ENV",
            "GIT_SSH_COMMAND",
            "PYTHONSTARTUP",
        ] {
            let rejected =
                service.set_variable("retrieval", reserved, "unsafe", "owner", &command_id()).await;
            assert_eq!(code(rejected), "invariant_violation", "{reserved}");
        }

        let rotated = service
            .set_secret("retrieval", "SERVICE_TOKEN", "rotated-value", "owner", &command_id())
            .await
            .unwrap();
        assert_eq!(rotated.configured, Some(true));
        assert_eq!(rotated.id, resource.id, "rotation updates the resource in place");
        assert_eq!(
            service.resource_environment("retrieval").await.unwrap()["SERVICE_TOKEN"],
            "rotated-value"
        );
        let named = service
            .list_resources("retrieval", true)
            .await
            .unwrap()
            .into_iter()
            .filter(|item| item.name == "SERVICE_TOKEN")
            .count();
        assert_eq!(named, 1);

        let removed = service
            .remove_resource("retrieval", "SERVICE_TOKEN", "owner", &command_id())
            .await
            .unwrap();
        assert_eq!(removed.configured, Some(false));
        assert!(
            !service.resource_environment("retrieval").await.unwrap().contains_key("SERVICE_TOKEN")
        );
        let mut tx = backend.db.begin().await.unwrap();
        let wiped: (Option<String>, Option<String>) = tx
            .fetch_one(
                "SELECT value, secret_ciphertext FROM project_resources \
                 WHERE name = 'SERVICE_TOKEN'",
                args![],
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(wiped, (None, None));

        let without_cipher = ChallengeService::new(backend.db.clone(), None);
        let unavailable =
            without_cipher.set_secret("retrieval", "OTHER", "value", "owner", &command_id()).await;
        assert_eq!(
            unavailable.unwrap_err().message,
            "This Limina instance has no secret-encryption provider."
        );
    })
    .await;
}

#[tokio::test]
async fn generated_secret_key_survives_instance_restart() {
    if std::env::var("LIMINA_SECRET_KEY").is_ok_and(|key| !key.trim().is_empty()) {
        return; // An operator key replaces the key file.
    }
    let dir = tempfile::tempdir().unwrap();
    let key_path = dir.path().join("persistent-secret.key");
    let first = SecretCipher::load(&key_path).unwrap();
    let ciphertext = first.encrypt("retrieval", "TOKEN", "durable");
    let second = SecretCipher::load(&key_path).unwrap();
    assert_eq!(second.decrypt("retrieval", "TOKEN", &ciphertext).unwrap(), "durable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&key_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn append_only_observations_do_not_contend_on_artifact_version() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let hypothesis = create_hypothesis(&service, "Reranking").await;
        let experiment = create_experiment(&service, &hypothesis.id, "Benchmark").await;
        service
            .claim_experiment("retrieval", &experiment.id, 300, "worker", &command_id())
            .await
            .unwrap();
        let tasks: Vec<_> = (0..20)
            .map(|index| {
                let service = service.clone();
                let artifact_id = experiment.id.clone();
                tokio::spawn(async move {
                    service
                        .append_observation(
                            "retrieval",
                            &artifact_id,
                            &format!("Observation {index}"),
                            Some(&format!("s3://evidence/{index}.json")),
                            "worker",
                            &command_id(),
                        )
                        .await
                        .unwrap();
                })
            })
            .collect();
        for task in tasks {
            task.await.unwrap();
        }
        let detailed = service.get_artifact("retrieval", &experiment.id).await.unwrap();
        assert_eq!(detailed.artifact.version, 2);
        assert_eq!(detailed.observations.unwrap().len(), 20);
        assert_eq!(detailed.lease.unwrap().owner, "worker");
    })
    .await;
}

#[tokio::test]
async fn stale_artifact_and_coordinator_writes_are_rejected() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let hypothesis = create_hypothesis(&service, "Reranking").await;
        let stale = service
            .decide_hypothesis(
                "retrieval",
                &hypothesis.id,
                HypothesisDecision {
                    status: "CONFIRMED".into(),
                    conclusion: "Stale decision.".into(),
                    expected_version: 99,
                },
                "reviewer",
                &command_id(),
            )
            .await;
        assert_eq!(code(stale), "version_conflict");

        let checkpoint = |worker: &str, ids: Vec<String>| Checkpoint {
            current_objective: "Run two lanes.".into(),
            next_step: "Wait for experiments.".into(),
            blocker: "None".into(),
            status: "RUNNING".into(),
            worker_id: Some(format!("coordinator-{worker}")),
            continuation_id: Some(format!("thread-{worker}")),
            inbox_cursor: 0,
            expected_version: 1,
            acknowledge_message_ids: ids,
            wake_at: None,
        };
        let saved = service
            .checkpoint_coordinator(
                "retrieval",
                checkpoint("a", vec![]),
                "coordinator-a",
                &command_id(),
            )
            .await
            .unwrap();
        assert_eq!(saved.version, 2);
        let pending = service
            .send_message(
                "retrieval",
                "COMMENT",
                "This must remain pending when the checkpoint conflicts.",
                None,
                "owner",
                &command_id(),
            )
            .await
            .unwrap();
        let conflict = service
            .checkpoint_coordinator(
                "retrieval",
                checkpoint("b", vec![pending.id.clone()]),
                "coordinator-b",
                &command_id(),
            )
            .await;
        assert_eq!(code(conflict), "version_conflict");
        let still_pending = service.inbox("retrieval", 0, true).await.unwrap();
        assert_eq!(still_pending.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), [pending.id]);
    })
    .await;
}

#[tokio::test]
async fn full_chain_completes_and_releases_the_lease() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let hypothesis = create_hypothesis(&service, "Reranking").await;
        let experiment = create_experiment(&service, &hypothesis.id, "Benchmark").await;
        let claimed = service
            .claim_experiment("retrieval", &experiment.id, 300, "worker", &command_id())
            .await
            .unwrap();
        service
            .append_observation(
                "retrieval",
                &experiment.id,
                "NDCG improved by twelve percent.",
                Some("s3://evidence/run.json"),
                "worker",
                &command_id(),
            )
            .await
            .unwrap();
        let completed = service
            .complete_experiment(
                "retrieval",
                &experiment.id,
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
        assert_eq!(completed.status, "COMPLETED");
        assert_eq!(
            completed.payload["completed_at"],
            limina_core::now().format("%Y-%m-%d").to_string()
        );
        let finding = service
            .publish_finding(
                "retrieval",
                NewFinding {
                    experiment_id: experiment.id.clone(),
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
        assert_eq!(finding.hypothesis_id.as_deref(), Some(hypothesis.id.as_str()));
        assert_eq!(finding.experiment_id.as_deref(), Some(experiment.id.as_str()));
        let detail = service.get_artifact("retrieval", &experiment.id).await.unwrap();
        assert!(detail.lease.is_none(), "completing releases the lease");
        let mut tx = backend.db.begin().await.unwrap();
        let revisions: Vec<(i32, String)> = tx
            .fetch_all(
                "SELECT version, status FROM artifact_revisions WHERE artifact_uid = $1 \
                 ORDER BY version",
                args![&experiment.uid],
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(
            revisions,
            [(1, "DESIGNED".to_owned()), (2, "RUNNING".to_owned()), (3, "COMPLETED".to_owned())]
        );
    })
    .await;
}
