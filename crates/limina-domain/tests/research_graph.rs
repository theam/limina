//! The H → E → F rules: leases, state transitions, revisions and artifact lookups.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod service_support;

use limina_domain::research::{ExperimentOutcome, HypothesisDecision};
use limina_store::{Db, args};
use serde_json::json;
use service_support::{
    code, command_id, create_experiment, create_hypothesis, each_backend, retrieval,
};

async fn expire_leases(db: &Db) {
    let mut tx = db.begin_write().await.unwrap();
    tx.execute(
        "UPDATE work_leases SET expires_at = $1",
        args![limina_core::now() - chrono::Duration::seconds(1)],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

fn outcome(expected_version: i32) -> ExperimentOutcome {
    ExperimentOutcome {
        results: "R.".into(),
        analysis: "A.".into(),
        decision: "D.".into(),
        expected_version,
    }
}

#[tokio::test]
async fn experiment_work_requires_an_active_lease_held_by_the_actor() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let h = create_hypothesis(&service, "Reranking").await;
        let e = create_experiment(&service, &h.id, "Benchmark").await;

        let early = service
            .append_observation("retrieval", &e.id, "Too early.", None, "worker", &command_id())
            .await
            .unwrap_err();
        assert_eq!(early.message, "Observations require a RUNNING experiment; E001 is DESIGNED.");
        let early = service
            .complete_experiment("retrieval", &e.id, outcome(1), "worker", &command_id())
            .await
            .unwrap_err();
        assert_eq!(early.message, "Only a RUNNING experiment can complete; E001 is DESIGNED.");

        let claim = service
            .claim_experiment("retrieval", "e001", 300, "worker", &command_id())
            .await
            .unwrap();
        assert_eq!((claim.artifact.status.as_str(), claim.artifact.version), ("RUNNING", 2));
        assert_eq!(claim.lease.version, 1);
        let renewed = service
            .claim_experiment("retrieval", "E001", 600, "worker", &command_id())
            .await
            .unwrap();
        assert_eq!(renewed.artifact.version, 2, "renewing does not touch the experiment");
        assert_eq!(renewed.lease.version, 2);
        assert!(renewed.lease.expires_at > claim.lease.expires_at);

        let mut tx = backend.db.begin_write().await.unwrap();
        tx.execute("DELETE FROM work_leases", args![]).await.unwrap();
        tx.commit().await.unwrap();
        let unclaimed = service
            .append_observation("retrieval", &e.id, "No lease.", None, "worker", &command_id())
            .await
            .unwrap_err();
        assert_eq!(unclaimed.code(), "lease_conflict");
        assert_eq!(unclaimed.message, "E001 must be claimed before it can run.");
        assert_eq!(unclaimed.details["artifact_id"], "E001");

        let claim = service
            .claim_experiment("retrieval", "E001", 300, "worker", &command_id())
            .await
            .unwrap();
        expire_leases(&backend.db).await;
        let expired = service
            .append_observation("retrieval", &e.id, "Late.", None, "worker", &command_id())
            .await
            .unwrap_err();
        assert_eq!(expired.code(), "lease_conflict");
        assert!(expired.message.starts_with("The lease for E001 expired at "));
        assert!(expired.details["expires_at"].as_str().unwrap().ends_with("+00:00"));
        assert!(service.get_artifact("retrieval", "E001").await.unwrap().lease.is_none());

        let takeover = service
            .claim_experiment("retrieval", "E001", 300, "rescuer", &command_id())
            .await
            .unwrap();
        assert_eq!(takeover.lease.owner, "rescuer");
        let stale_owner = service
            .complete_experiment(
                "retrieval",
                &e.id,
                outcome(claim.artifact.version),
                "worker",
                &command_id(),
            )
            .await
            .unwrap_err();
        assert_eq!(stale_owner.message, "E001 is claimed by rescuer; current actor is worker.");
        assert_eq!(stale_owner.details["owner"], "rescuer");
        assert_eq!(stale_owner.details["actor"], "worker");

        service
            .complete_experiment("retrieval", &e.id, outcome(2), "rescuer", &command_id())
            .await
            .unwrap();
        let reclaim = service
            .claim_experiment("retrieval", "E001", 300, "rescuer", &command_id())
            .await
            .unwrap_err();
        assert_eq!(reclaim.message, "Cannot claim E001 while it is COMPLETED.");
        assert_eq!(reclaim.details["status"], "COMPLETED");
        let bad_ttl =
            service.claim_experiment("retrieval", "E001", 86_401, "rescuer", &command_id()).await;
        assert_eq!(code(bad_ttl), "invariant_violation");
    })
    .await;
}

#[tokio::test]
async fn every_artifact_change_is_a_revision() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let h = create_hypothesis(&service, "Reranking").await;
        create_experiment(&service, &h.id, "Benchmark").await;
        let decided = service
            .decide_hypothesis(
                "retrieval",
                "h001",
                HypothesisDecision {
                    status: "rejected".into(),
                    conclusion: "  No gain.  ".into(),
                    expected_version: 2,
                },
                "reviewer",
                &command_id(),
            )
            .await
            .unwrap();
        assert_eq!((decided.status.as_str(), decided.version), ("REJECTED", 3));
        assert_eq!(decided.payload["conclusion"], "No gain.");
        assert_eq!(
            serde_json::to_string(&decided.payload).unwrap(),
            serde_json::to_string(&json!({
                "statement": "A stronger reranker improves NDCG.",
                "mechanism": "Pairwise relevance modeling.",
                "generalization": "The mechanism is query independent.",
                "shortcut_risks": "Overfitting the fixture.",
                "test_plan": "Use a frozen candidate set.",
                "conclusion": "No gain.",
            }))
            .unwrap(),
            "payload keys keep their order"
        );
        let closed = service
            .create_experiment(
                "retrieval",
                service_support::experiment("H001", "Too late"),
                "researcher",
                &command_id(),
            )
            .await
            .unwrap_err();
        assert_eq!(closed.message, "Cannot add an experiment to H001 because it is REJECTED.");

        let mut tx = backend.db.begin().await.unwrap();
        let revisions: Vec<(i32, String, String, String)> = tx
            .fetch_all(
                "SELECT version, status, actor, CAST(payload AS TEXT) FROM artifact_revisions \
                 WHERE artifact_uid = $1 ORDER BY version",
                args![&h.uid],
            )
            .await
            .unwrap();
        let events: Vec<(String, Option<String>)> = tx
            .fetch_all("SELECT event_type, artifact_id FROM events ORDER BY sequence", args![])
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let summary: Vec<(i32, &str, &str)> =
            revisions.iter().map(|(v, s, a, _)| (*v, s.as_str(), a.as_str())).collect();
        assert_eq!(
            summary,
            [
                (1, "PROPOSED", "researcher"),
                (2, "TESTING", "researcher"),
                (3, "REJECTED", "reviewer")
            ]
        );
        assert!(revisions[2].3.contains("No gain."));
        assert!(!revisions[1].3.contains("No gain."), "each revision is a snapshot");
        assert_eq!(
            events,
            [
                ("challenge.created".to_owned(), None),
                ("hypothesis.created".to_owned(), Some("H001".to_owned())),
                ("experiment.created".to_owned(), Some("E001".to_owned())),
                ("hypothesis.decided".to_owned(), Some("H001".to_owned())),
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn artifacts_are_looked_up_by_display_id() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let h = create_hypothesis(&service, "Reranking").await;
        let detail =
            serde_json::to_value(service.get_artifact("retrieval", "h001").await.unwrap()).unwrap();
        assert!(detail.get("observations").is_none() && detail.get("lease").is_none());
        let e = create_experiment(&service, &h.id, "Benchmark").await;
        let detail =
            serde_json::to_value(service.get_artifact("retrieval", &e.id).await.unwrap()).unwrap();
        assert_eq!(detail["observations"], json!([]));
        assert!(detail.get("lease").is_none(), "no lease until claimed");

        let error = service.get_artifact("retrieval", "x1").await.unwrap_err();
        assert_eq!(
            (error.message.as_str(), &error.details["artifact_id"]),
            ("Artifact 'x1' does not exist.", &json!("x1"))
        );
        let error = service.get_artifact("retrieval", "cr001").await.unwrap_err();
        assert_eq!(error.message, "Artifact 'CR001' does not exist.");
        let error = service.get_artifact("missing", "bogus").await.unwrap_err();
        assert_eq!(error.message, "Artifact 'bogus' does not exist.", "the ID is checked first");

        assert_eq!(service.list_artifacts("retrieval", Some("e")).await.unwrap().len(), 1);
        assert_eq!(service.list_artifacts("retrieval", Some("")).await.unwrap().len(), 2);
        assert!(service.list_artifacts("retrieval", Some("F")).await.unwrap().is_empty());

        // Display IDs keep at least three digits and grow past them.
        let mut tx = backend.db.begin_write().await.unwrap();
        tx.execute("UPDATE id_counters SET next_value = 1000 WHERE kind = 'H'", args![])
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let big = create_hypothesis(&service, "Thousandth").await;
        assert_eq!(big.id, "H1000");
        assert_eq!(service.get_artifact("retrieval", "h1000").await.unwrap().artifact.uid, big.uid);
        let ids: Vec<String> = service
            .list_artifacts("retrieval", None)
            .await
            .unwrap()
            .into_iter()
            .map(|item| item.id)
            .collect();
        assert_eq!(ids, ["E001", "H001", "H1000"], "ordered by display ID text");
    })
    .await;
}

#[tokio::test]
async fn findings_link_their_experiment_and_hypothesis() {
    each_backend(|backend| async move {
        let service = backend.service();
        retrieval(&service).await;
        let h = create_hypothesis(&service, "Reranking").await;
        let e = create_experiment(&service, &h.id, "Benchmark").await;
        service.claim_experiment("retrieval", &e.id, 300, "worker", &command_id()).await.unwrap();
        let completed = service
            .complete_experiment("retrieval", &e.id, outcome(2), "worker", &command_id())
            .await
            .unwrap();
        assert_eq!(completed.version, 3);
        assert_eq!(
            completed.payload.as_object().unwrap().keys().collect::<Vec<_>>(),
            [
                "objective",
                "procedure",
                "success_criteria",
                "guardrails",
                "results",
                "analysis",
                "decision",
                "completed_at"
            ]
        );
        let finding = service
            .publish_finding(
                "retrieval",
                limina_domain::research::NewFinding {
                    experiment_id: "e001".into(),
                    title: " Finding ".into(),
                    finding: "F.".into(),
                    evidence: "E.".into(),
                    impact: "critical".into(),
                    ..Default::default()
                },
                "worker",
                &command_id(),
            )
            .await
            .unwrap();
        assert_eq!(finding.id, "F001");
        assert_eq!(finding.title, "Finding");
        assert_eq!(finding.status, "PUBLISHED");
        assert_eq!(finding.payload["impact"], "CRITICAL");
        let events = service.recent_events("retrieval", 1).await.unwrap();
        assert_eq!(
            events[0].payload,
            json!({"hypothesis_id": "H001", "experiment_id": "E001", "impact": "CRITICAL"})
        );
        let not_experiment = service
            .publish_finding(
                "retrieval",
                limina_domain::research::NewFinding {
                    experiment_id: "H001".into(),
                    title: "x".into(),
                    finding: "x".into(),
                    evidence: "x".into(),
                    impact: "LOW".into(),
                    ..Default::default()
                },
                "worker",
                &command_id(),
            )
            .await;
        assert_eq!(code(not_experiment), "not_found");
    })
    .await;
}
