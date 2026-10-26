#![allow(clippy::unwrap_used)]
mod collaboration_support;
use collaboration_support::*;
use limina_domain::exporter::MarkdownExporter;
use limina_domain::knowledge::KnowledgeQuery;
use limina_domain::progress::MetricInput;
use limina_domain::service::ChallengeService;
use serde_json::json;

async fn seed(backend: &Backend, slug: &str) -> String {
    let project = create_project(&backend.db, slug, Some(&owner())).await;
    for (id, kind, status, parent, experiment, payload) in [
        (
            "H001",
            "H",
            "TESTING",
            None,
            None,
            json!({"statement":"A complete, long hypothesis statement."}),
        ),
        (
            "E001",
            "E",
            "COMPLETED",
            Some("H001"),
            None,
            json!({"objective":"Measure held-out precision.","results":"Precision reaches 92%."}),
        ),
        (
            "E002",
            "E",
            "RUNNING",
            Some("H001"),
            None,
            json!({"objective":"An unfinished measurement."}),
        ),
        (
            "F001",
            "F",
            "PUBLISHED",
            Some("H001"),
            Some("E001"),
            json!({"finding":"Precision is 92%.","evidence":"Held-out test split.","impact":"HIGH"}),
        ),
    ] {
        insert_artifact(
            &backend.db,
            &project,
            ArtifactRow {
                id,
                kind,
                status,
                hypothesis: parent,
                experiment,
                ..ArtifactRow::hypothesis(id, "A readable title", payload, at(20, 12, 0))
            },
        )
        .await;
    }
    project
}
fn definition() -> MetricInput {
    MetricInput {
        name: "Precision".into(),
        criterion: "Precision at least 95% on the held-out split".into(),
        unit: "%".into(),
        direction: "at_least".into(),
        target: 95.,
        baseline: Some(70.),
        ..Default::default()
    }
}

#[tokio::test]
async fn kpis_are_durable_versioned_idempotent_and_require_real_project_evidence() {
    for backend in backends().await {
        seed(&backend, "tracked").await;
        seed(&backend, "other").await;
        let service = backend.service();
        let created = service
            .set_metric("tracked", "precision", definition(), 0, "researcher", "new-kpi")
            .await
            .unwrap();
        assert_eq!(created.input.current, None);
        assert_eq!(created.input.observed_at, None);
        assert_eq!(
            service
                .set_metric("tracked", "precision", definition(), 0, "researcher", "new-kpi")
                .await
                .unwrap(),
            created
        );
        let measured =
            MetricInput { current: Some(92.), evidence_id: Some("f001".into()), ..definition() };
        let result = service
            .set_metric("tracked", "precision", measured.clone(), 1, "researcher", "measure-kpi")
            .await
            .unwrap();
        assert_eq!(result.version, 2);
        assert!(result.input.observed_at.is_some());
        assert_eq!(result.input.evidence_id.as_deref(), Some("F001"));
        assert_eq!(
            service.metrics("tracked").await.unwrap().as_slice(),
            std::slice::from_ref(&result)
        );
        let error = service
            .set_metric("tracked", "precision", measured.clone(), 1, "researcher", "stale-edit")
            .await
            .unwrap_err();
        assert_eq!(error.http_status(), 409);
        assert!(service.metrics("other").await.unwrap().is_empty());
        for (evidence, status) in
            [(None, 422), (Some("H001"), 422), (Some("E002"), 422), (Some("F999"), 404)]
        {
            let input = MetricInput {
                current: Some(90.),
                evidence_id: evidence.map(str::to_owned),
                ..definition()
            };
            assert_eq!(
                service
                    .set_metric(
                        "tracked",
                        "bad-evidence",
                        input,
                        0,
                        "researcher",
                        &limina_core::new_uuid()
                    )
                    .await
                    .unwrap_err()
                    .http_status(),
                status
            );
        }
        let mut changed = result.input.clone();
        changed.name = "Renamed precision".into();
        changed.observed_at = None;
        let renamed = service
            .set_metric("tracked", "precision", changed, 2, "researcher", "rename-kpi")
            .await
            .unwrap();
        assert_eq!(
            renamed.input.observed_at, result.input.observed_at,
            "editing a definition must not pretend to take a new measurement"
        );
        let mut tx = backend.db.begin().await.unwrap();
        let events:i64=tx.scalar("SELECT CAST(COUNT(*) AS BIGINT) FROM events WHERE event_type = 'project.metric.updated'",vec![]).await.unwrap();
        assert_eq!(events, 3, "retries and rejected writes must not add audit events");
        tx.commit().await.unwrap();
    }
}

#[tokio::test]
async fn focused_notes_and_parent_queries_do_not_expand_the_entire_graph() {
    for backend in backends().await {
        seed(&backend, "focused").await;
        let service = backend.service();
        let page = service
            .query_knowledge(
                "focused",
                &KnowledgeQuery { parent: Some("h001".into()), limit: 1, ..Default::default() },
            )
            .await
            .unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(page.items.len(), 1);
        assert!(page.next_cursor.is_some());
        let by_id = service
            .query_knowledge(
                "focused",
                &KnowledgeQuery { query: Some("h001".into()), ..Default::default() },
            )
            .await
            .unwrap();
        assert_eq!(by_id.total, 1);
        assert_eq!(by_id.items[0].id, "H001");
        let exporter = MarkdownExporter::new(ChallengeService::new(backend.db.clone(), None));
        let note = exporter.document("focused", "F001").await.unwrap();
        assert_eq!(note.len(), 1);
        let body = note.values().next().unwrap();
        assert!(body.contains("Precision is 92%."));
        assert!(body.contains("[[H001]]"));
        assert!(body.contains("[[E001]]"));
        let full = exporter.snapshot("focused").await.unwrap();
        assert_eq!(full.get(note.keys().next().unwrap()).unwrap(), body);
        assert_eq!(exporter.document("focused", "CHALLENGE").await.unwrap().len(), 1);
        assert!(exporter.document("focused", "F999").await.is_err());
    }
}
