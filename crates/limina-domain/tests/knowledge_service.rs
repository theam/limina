//! Knowledge search, tags, relations and the graph, comments, revisions, saved views and sources.
//! Ported from `test_kickoff_knowledge_sources_runs_and_analytics_are_queryable` in
//! `tests/test_cloud_ui_readiness.py` at the service level.

#![allow(clippy::unwrap_used)]

mod collaboration_support;

use collaboration_support::*;
use limina_domain::collaboration::CollaborationService;
use limina_domain::knowledge::{KnowledgeQuery, NewRelation, SourceInput};
use limina_store::{Db, args};
use serde_json::{Map, Value, json};

fn keys(value: &Value) -> Vec<&str> {
    value.as_object().unwrap().keys().map(String::as_str).collect()
}

fn search(query: Option<&str>) -> KnowledgeQuery {
    KnowledgeQuery { query: query.map(str::to_owned), ..KnowledgeQuery::default() }
}

fn object(value: Value) -> Map<String, Value> {
    serde_json::from_value(value).unwrap()
}

/// Two hypotheses like the Python test creates, plus a finding in another kind and status.
async fn research_project(db: &Db, slug: &str) -> String {
    let id = create_project(db, slug, Some(&owner())).await;
    let hypothesis = |statement: &str, mechanism: &str| {
        json!({
            "statement": statement,
            "mechanism": mechanism,
            "generalization": "Across artifacts",
            "shortcut_risks": "Vocabulary mismatch",
            "test_plan": "Query known terms",
        })
    };
    let first = hypothesis("Full-text retrieval narrows the graph.", "Lexical matching");
    let second = hypothesis("Explicit links improve review.", "Backlinks");
    insert_artifact(
        db,
        &id,
        ArtifactRow::hypothesis("H001", "Hybrid retrieval", first, at(2, 9, 0)),
    )
    .await;
    insert_artifact(
        db,
        &id,
        ArtifactRow::hypothesis("H002", "Graph navigation", second, at(2, 10, 0)),
    )
    .await;
    id
}

/// The search part of `test_kickoff_knowledge_sources_runs_and_analytics_are_queryable`.
#[tokio::test]
async fn knowledge_search_filters_and_pages() {
    for backend in backends().await {
        let service = backend.service();
        let id = research_project(&backend.db, "ui-ready").await;
        // Wildcards are literal text, never patterns (`?query=%25` finds nothing).
        for literal in ["%", "/"] {
            let page = service.query_knowledge("ui-ready", &search(Some(literal))).await.unwrap();
            assert_eq!(page.total, 0, "{}: {literal:?}", backend.name);
        }
        insert_artifact(
            &backend.db,
            &id,
            ArtifactRow {
                kind: "F",
                status: "ACCEPTED",
                hypothesis: Some("H001"),
                ..ArtifactRow::hypothesis(
                    "F001",
                    "Retrieval 50% faster",
                    json!({"claim": "100 000 queries"}),
                    at(2, 11, 0),
                )
            },
        )
        .await;

        let found =
            service.query_knowledge("ui-ready", &search(Some(" retrieval "))).await.unwrap();
        let backend_name =
            if backend.is_postgres() { "postgresql-fts" } else { "portable-substring" };
        assert_eq!(found.search_backend, backend_name);
        let ids: Vec<&str> = found.items.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(found.total, 2, "{}: {ids:?}", backend.name);
        assert!(ids.contains(&"H001") && ids.contains(&"F001"));
        let content = service.query_knowledge("ui-ready", &search(Some("lexical"))).await.unwrap();
        assert_eq!(content.items.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(), ["H001"]);
        let json = serde_json::to_value(&content).unwrap();
        assert_eq!(keys(&json), ["items", "next_cursor", "total", "search_backend"]);
        assert_eq!(
            keys(&json["items"][0]),
            [
                "id",
                "kind",
                "title",
                "status",
                "content",
                "hypothesis_id",
                "experiment_id",
                "created_at",
                "updated_at",
                "tags"
            ]
        );
        assert_eq!(json["items"][0]["content"]["mechanism"], "Lexical matching");
        assert_eq!(json["items"][0]["created_at"], "2026-10-02T09:00:00+00:00");

        if !backend.is_postgres() {
            // Substring search matches `%` and `_` literally (`_` appears in payload keys).
            assert_eq!(
                service.query_knowledge("ui-ready", &search(Some("50%"))).await.unwrap().total,
                1
            );
            assert_eq!(
                service.query_knowledge("ui-ready", &search(Some("0%"))).await.unwrap().total,
                1
            );
            assert_eq!(
                service.query_knowledge("ui-ready", &search(Some("t_p"))).await.unwrap().total,
                2
            );
            assert_eq!(
                service.query_knowledge("ui-ready", &search(Some("1_0"))).await.unwrap().total,
                0
            );
        }

        // Most recently updated first, one page at a time.
        let first = service
            .query_knowledge("ui-ready", &KnowledgeQuery { limit: 1, ..KnowledgeQuery::default() })
            .await
            .unwrap();
        assert_eq!(
            (first.total, first.items[0].id.as_str(), first.next_cursor.as_deref()),
            (3, "F001", Some("MQ"))
        );
        let second = service
            .query_knowledge(
                "ui-ready",
                &KnowledgeQuery {
                    limit: 1,
                    cursor: first.next_cursor.clone(),
                    ..KnowledgeQuery::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            (second.items[0].id.as_str(), second.next_cursor.as_deref()),
            ("H002", Some("Mg"))
        );
        let last = service
            .query_knowledge(
                "ui-ready",
                &KnowledgeQuery { cursor: Some("Mg".to_owned()), ..KnowledgeQuery::default() },
            )
            .await
            .unwrap();
        assert_eq!((last.items.len(), last.next_cursor), (1, None));

        let hypotheses = KnowledgeQuery {
            kind: Some("h".to_owned()),
            status: Some("proposed".to_owned()),
            ..KnowledgeQuery::default()
        };
        assert_eq!(service.query_knowledge("ui-ready", &hypotheses).await.unwrap().total, 2);
        let empty_filters = KnowledgeQuery {
            query: Some("  ".to_owned()),
            kind: Some(String::new()),
            status: Some(String::new()),
            tag: Some(String::new()),
            ..KnowledgeQuery::default()
        };
        assert_eq!(service.query_knowledge("ui-ready", &empty_filters).await.unwrap().total, 3);

        // `test_kickoff_...`: tag filters return tagged artifacts with their tags.
        assert_eq!(
            service.add_tag("ui-ready", "h001", "Retrieval", "Olivia Owner").await.unwrap(),
            ["retrieval"]
        );
        let tagged =
            KnowledgeQuery { tag: Some(" RETRIEVAL ".to_owned()), ..KnowledgeQuery::default() };
        let tagged = service.query_knowledge("ui-ready", &tagged).await.unwrap();
        assert_eq!((tagged.total, tagged.items[0].tags.clone()), (1, vec!["retrieval".to_owned()]));
        let bad_tag = KnowledgeQuery { tag: Some("a/b".to_owned()), ..KnowledgeQuery::default() };
        let error = service.query_knowledge("ui-ready", &bad_tag).await.unwrap_err();
        assert_eq!(
            error.message,
            "Knowledge tags must use letters, numbers, dots, underscores, or hyphens."
        );
        let bad_cursor =
            KnowledgeQuery { cursor: Some("M".to_owned()), ..KnowledgeQuery::default() };
        let error = service.query_knowledge("ui-ready", &bad_cursor).await.unwrap_err();
        assert_eq!(
            (error.code(), error.message.as_str()),
            ("invariant_violation", "The pagination cursor is invalid.")
        );
        let missing =
            service.query_knowledge("missing", &KnowledgeQuery::default()).await.unwrap_err();
        assert_eq!(missing.message, "Project 'missing' does not exist.");
    }
}

#[tokio::test]
async fn tags_are_normalized_and_listed() {
    for backend in backends().await {
        let service = backend.service();
        let id = research_project(&backend.db, "ui-ready").await;

        assert_eq!(
            service.add_tag("ui-ready", "H001", " Retrieval Eval ", "Erin Editor").await.unwrap(),
            ["retrieval-eval"]
        );
        assert_eq!(
            service.add_tag("ui-ready", "h001", "retrieval-eval", "Erin Editor").await.unwrap(),
            ["retrieval-eval"]
        );
        assert_eq!(
            service.add_tag("ui-ready", "H001", "baseline", "Erin Editor").await.unwrap(),
            ["baseline", "retrieval-eval"]
        );
        assert_eq!(service.tags("ui-ready", "h001").await.unwrap(), ["baseline", "retrieval-eval"]);
        assert!(service.tags("ui-ready", "H002").await.unwrap().is_empty());
        assert_eq!(
            service.remove_tag("ui-ready", "H001", "Baseline", "Erin Editor").await.unwrap(),
            ["retrieval-eval"]
        );

        let missing =
            service.remove_tag("ui-ready", "H001", "baseline", "Erin Editor").await.unwrap_err();
        assert_eq!(
            (missing.code(), missing.message.as_str()),
            ("not_found", "Knowledge tag does not exist.")
        );
        assert_eq!(Value::Object(missing.details), json!({"tag": "baseline"}));
        let invalid = service.add_tag("ui-ready", "H001", "a/b", "Erin Editor").await.unwrap_err();
        assert_eq!(invalid.code(), "invariant_violation");
        let unknown = service.add_tag("ui-ready", "h999", "x", "Erin Editor").await.unwrap_err();
        assert_eq!(
            (unknown.code(), unknown.message.as_str()),
            ("not_found", "Knowledge artifact 'h999' does not exist.")
        );
        assert!(unknown.details.is_empty());

        // Adding an existing tag records nothing.
        let events: Vec<(String, Option<String>, Value)> = event_log(&backend.db, &id)
            .await
            .into_iter()
            .map(|(kind, _, artifact, payload)| (kind, artifact, payload))
            .collect();
        assert_eq!(
            events,
            [
                (
                    "knowledge.tag_added".to_owned(),
                    Some("H001".to_owned()),
                    json!({"tag": "retrieval-eval"})
                ),
                (
                    "knowledge.tag_added".to_owned(),
                    Some("H001".to_owned()),
                    json!({"tag": "baseline"})
                ),
                (
                    "knowledge.tag_removed".to_owned(),
                    Some("H001".to_owned()),
                    json!({"tag": "baseline"})
                ),
            ]
        );
    }
}

/// The relation and graph part of `test_kickoff_knowledge_sources_runs_and_analytics_are_queryable`,
/// with the graph checked against the Python runtime's output for the same rows.
#[tokio::test]
async fn relations_and_the_knowledge_graph() {
    for backend in backends().await {
        let service = backend.service();
        let id = research_project(&backend.db, "ui-ready").await;
        insert_artifact(
            &backend.db,
            &id,
            ArtifactRow {
                kind: "E",
                status: "RUNNING",
                hypothesis: Some("H001"),
                ..ArtifactRow::hypothesis("E001", "Query known terms", json!({}), at(3, 9, 0))
            },
        )
        .await;
        insert_artifact(
            &backend.db,
            &id,
            ArtifactRow {
                kind: "F",
                status: "ACCEPTED",
                hypothesis: Some("H001"),
                experiment: Some("E001"),
                ..ArtifactRow::hypothesis(
                    "F001",
                    "Lexical recall holds",
                    json!({"claim": "Recall 0.91"}),
                    at(3, 10, 0),
                )
            },
        )
        .await;
        service.add_tag("ui-ready", "F001", "evidence", "Olivia Owner").await.unwrap();

        let relation = NewRelation {
            source_id: "H001".to_owned(),
            target_id: "H002".to_owned(),
            relation_type: " supports ".to_owned(),
            description: "  Search narrows navigation. ".to_owned(),
        };
        let created = service.create_relation("ui-ready", &relation, "Olivia Owner").await.unwrap();
        assert_eq!(
            (created.relation_type.as_str(), created.description.as_str()),
            ("SUPPORTS", "Search narrows navigation.")
        );
        let json = serde_json::to_value(&created).unwrap();
        assert_eq!(
            keys(&json),
            [
                "id",
                "source_id",
                "target_id",
                "type",
                "description",
                "derived",
                "created_by",
                "created_at"
            ]
        );
        assert_eq!(
            (json["derived"].clone(), json["created_by"].clone()),
            (json!(false), json!("Olivia Owner"))
        );

        let spaced =
            NewRelation { relation_type: "contradicts  in part".to_owned(), ..relation.clone() };
        let spaced = service.create_relation("ui-ready", &spaced, "Olivia Owner").await.unwrap();
        assert_eq!(spaced.relation_type, "CONTRADICTS__IN_PART");
        let duplicate =
            service.create_relation("ui-ready", &relation, "Olivia Owner").await.unwrap_err();
        assert_eq!(
            (duplicate.code(), duplicate.message.as_str()),
            ("version_conflict", "That knowledge relation already exists.")
        );
        // Stored IDs are taken as given, so a lower-case ID breaks the foreign key like in Python.
        let lower = NewRelation {
            source_id: "h002".to_owned(),
            target_id: "H001".to_owned(),
            ..relation.clone()
        };
        let lower = service.create_relation("ui-ready", &lower, "Olivia Owner").await.unwrap_err();
        assert_eq!(lower.message, "That knowledge relation already exists.");
        for (invalid, message) in [
            (
                NewRelation { relation_type: " ".to_owned(), ..relation.clone() },
                "Relation type must be 1 to 80 characters.",
            ),
            (
                NewRelation { relation_type: "x".repeat(81), ..relation.clone() },
                "Relation type must be 1 to 80 characters.",
            ),
            (
                NewRelation { target_id: "H001".to_owned(), ..relation.clone() },
                "A knowledge relation cannot point to itself.",
            ),
        ] {
            let error =
                service.create_relation("ui-ready", &invalid, "Olivia Owner").await.unwrap_err();
            assert_eq!((error.code(), error.message.as_str()), ("invariant_violation", message));
        }
        let unknown = NewRelation { target_id: "H404".to_owned(), ..relation.clone() };
        let unknown =
            service.create_relation("ui-ready", &unknown, "Olivia Owner").await.unwrap_err();
        assert_eq!(unknown.message, "Knowledge artifact 'H404' does not exist.");

        let graph = service.graph("ui-ready").await.unwrap();
        let mut json = serde_json::to_value(&graph).unwrap();
        assert_eq!(keys(&json), ["nodes", "edges"]);
        let nodes: Vec<&str> = graph.nodes.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(nodes, ["E001", "F001", "H001", "H002"]);
        assert_eq!(json["nodes"][1]["tags"], json!(["evidence"]));
        for edge in json["edges"].as_array_mut().unwrap() {
            if edge["derived"] == json!(false) {
                edge["id"] = Value::Null;
                edge["created_at"] = Value::Null;
            }
        }
        assert_eq!(
            json["edges"],
            json!([
                {"id": null, "source_id": "H001", "target_id": "H002", "type": "SUPPORTS",
                 "description": "Search narrows navigation.", "derived": false, "created_by": "Olivia Owner", "created_at": null},
                {"id": null, "source_id": "H001", "target_id": "H002", "type": "CONTRADICTS__IN_PART",
                 "description": "Search narrows navigation.", "derived": false, "created_by": "Olivia Owner", "created_at": null},
                {"id": "derived:H001:TESTED_BY:E001", "source_id": "H001", "target_id": "E001", "type": "TESTED_BY",
                 "description": "", "derived": true, "created_by": "Limina", "created_at": null},
                {"id": "derived:H001:TESTED_BY:F001", "source_id": "H001", "target_id": "F001", "type": "TESTED_BY",
                 "description": "", "derived": true, "created_by": "Limina", "created_at": null},
                {"id": "derived:E001:FOUND:F001", "source_id": "E001", "target_id": "F001", "type": "FOUND",
                 "description": "", "derived": true, "created_by": "Limina", "created_at": null},
            ])
        );

        let removed =
            service.delete_relation("ui-ready", &created.id, "Erin Editor").await.unwrap();
        assert_eq!(removed, created);
        let again =
            service.delete_relation("ui-ready", &created.id, "Erin Editor").await.unwrap_err();
        assert_eq!(
            (again.code(), again.message.as_str()),
            ("not_found", "Knowledge relation does not exist.")
        );
        assert_eq!(service.graph("ui-ready").await.unwrap().edges.len(), 4);

        let events: Vec<(String, String, Option<String>, Value)> = event_log(&backend.db, &id)
            .await
            .into_iter()
            .filter(|event| event.0.contains("relation"))
            .collect();
        assert_eq!(events.len(), 3);
        assert_eq!(
            events[0],
            (
                "knowledge.relation_created".to_owned(),
                "Olivia Owner".to_owned(),
                Some("H001".to_owned()),
                json!({"relation_id": created.id, "source_id": "H001", "target_id": "H002"})
            )
        );
        assert_eq!(
            events[2],
            (
                "knowledge.relation_removed".to_owned(),
                "Erin Editor".to_owned(),
                Some("H001".to_owned()),
                json!({"relation_id": created.id})
            )
        );
    }
}

/// The comment part of `test_kickoff_knowledge_sources_runs_and_analytics_are_queryable`: comments
/// are idempotent per command ID and a reused key never leaks another project's comment.
#[tokio::test]
async fn comments_are_idempotent_and_tenant_scoped() {
    for backend in backends().await {
        let service = backend.service();
        let id = research_project(&backend.db, "ui-ready").await;
        let other = research_project(&backend.db, "comment-boundary").await;

        let first = service
            .add_comment(
                "ui-ready",
                "h001",
                " Please test vocabulary drift. ",
                "Olivia Owner",
                "key-1",
            )
            .await
            .unwrap();
        assert_eq!(
            (first.artifact_id.as_str(), first.body.as_str()),
            ("H001", "Please test vocabulary drift.")
        );
        let json = serde_json::to_value(&first).unwrap();
        assert_eq!(keys(&json), ["id", "artifact_id", "body", "actor", "created_at"]);
        let replayed = service
            .add_comment("ui-ready", "H001", "A different body.", "Olivia Owner", "key-1")
            .await
            .unwrap();
        assert_eq!(replayed, first);

        let original = service
            .add_comment(
                "ui-ready",
                "H001",
                "Private project comment.",
                "Olivia Owner",
                "predictable",
            )
            .await
            .unwrap();
        let collision = service
            .add_comment(
                "comment-boundary",
                "H001",
                "A different comment.",
                "Olivia Owner",
                "predictable",
            )
            .await
            .unwrap_err();
        assert_eq!((collision.code(), collision.http_status()), ("version_conflict", 409));
        assert_eq!(collision.message, "The idempotency key was already used for another comment.");
        assert_eq!(Value::Object(collision.details.clone()), json!({"command_id": "predictable"}));
        assert!(!format!("{:?}", collision.to_body()).contains("Private project comment"));
        for (artifact, actor) in [("H002", "Olivia Owner"), ("H001", "Erin Editor")] {
            let error = service
                .add_comment("ui-ready", artifact, "x", actor, "predictable")
                .await
                .unwrap_err();
            assert_eq!(error.code(), "version_conflict");
        }

        assert_eq!(
            service
                .add_comment("ui-ready", "H001", " \n", "Olivia Owner", "key-2")
                .await
                .unwrap_err()
                .message,
            "Comment cannot be empty."
        );
        let long = "x".repeat(32_769);
        assert_eq!(
            service
                .add_comment("ui-ready", "H001", &long, "Olivia Owner", "key-3")
                .await
                .unwrap_err()
                .message,
            "Comment must be at most 32768 characters."
        );
        assert!(
            service
                .add_comment("ui-ready", "H001", &"é".repeat(32_768), "Olivia Owner", "key-4")
                .await
                .is_ok()
        );

        let listed: Vec<String> = service
            .comments("ui-ready", "h001")
            .await
            .unwrap()
            .into_iter()
            .map(|comment| comment.id)
            .collect();
        assert_eq!(listed.len(), 3);
        assert_eq!(listed[..2], [first.id.clone(), original.id.clone()]);
        assert!(service.comments("comment-boundary", "H001").await.unwrap().is_empty());

        let events = event_log(&backend.db, &id).await;
        assert_eq!(events.len(), 3, "a replay records nothing");
        assert_eq!(
            events[0],
            (
                "knowledge.comment_added".to_owned(),
                "Olivia Owner".to_owned(),
                Some("H001".to_owned()),
                json!({"comment_id": first.id})
            )
        );
        let mut tx = backend.db.begin().await.unwrap();
        let command: String = tx
            .scalar(
                "SELECT command_id FROM events WHERE challenge_id = $1 ORDER BY sequence LIMIT 1",
                args![&id],
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(command, "key-1");
        assert!(event_log(&backend.db, &other).await.is_empty());

        // Concurrent retries of one command produce one comment.
        let service_a = service.clone();
        let service_b = service.clone();
        let (a, b) = tokio::join!(
            tokio::spawn(async move {
                service_a.add_comment("ui-ready", "H002", "Racing.", "Olivia Owner", "race").await
            }),
            tokio::spawn(async move {
                service_b.add_comment("ui-ready", "H002", "Racing.", "Olivia Owner", "race").await
            }),
        );
        assert_eq!(a.unwrap().unwrap().id, b.unwrap().unwrap().id);
        assert_eq!(service.comments("ui-ready", "H002").await.unwrap().len(), 1);
    }
}

/// The revision part of `test_kickoff_knowledge_sources_runs_and_analytics_are_queryable`.
#[tokio::test]
async fn revisions_are_listed_newest_first() {
    for backend in backends().await {
        let service = backend.service();
        let id = create_project(&backend.db, "ui-ready", Some(&owner())).await;
        let uid = insert_artifact(
            &backend.db,
            &id,
            ArtifactRow::hypothesis(
                "H001",
                "Hybrid retrieval",
                json!({"statement": "v1"}),
                at(2, 9, 0),
            ),
        )
        .await;
        assert_eq!(service.revisions("ui-ready", "H001").await.unwrap()[0].version, 1);
        insert_revision(&backend.db, &uid, 2, "TESTING", json!({"statement": "v2"})).await;

        let revisions = service.revisions("ui-ready", "h001").await.unwrap();
        assert_eq!(revisions.iter().map(|revision| revision.version).collect::<Vec<_>>(), [2, 1]);
        let json = serde_json::to_value(&revisions[1]).unwrap();
        assert_eq!(
            json,
            json!({"version": 1, "status": "PROPOSED", "title": "Hybrid retrieval", "content": {"statement": "v1"},
                   "actor": "Limina", "created_at": "2026-10-02T09:00:00+00:00"})
        );
        assert_eq!(keys(&json), ["version", "status", "title", "content", "actor", "created_at"]);
        let unknown = service.revisions("ui-ready", "H404").await.unwrap_err();
        assert_eq!(unknown.message, "Knowledge artifact 'H404' does not exist.");
    }
}

/// The saved-view part of `test_kickoff_knowledge_sources_runs_and_analytics_are_queryable`.
#[tokio::test]
async fn saved_views_are_replaced_by_name() {
    for backend in backends().await {
        let service = backend.service();
        let id = create_project(&backend.db, "ui-ready", Some(&owner())).await;

        let open = object(json!({"kind": "H", "status": "PROPOSED"}));
        let saved = service
            .save_view("ui-ready", " Open hypotheses ", &open, "Olivia Owner")
            .await
            .unwrap();
        assert_eq!((saved.name.as_str(), &saved.query), ("Open hypotheses", &open));
        let json = serde_json::to_value(&saved).unwrap();
        assert_eq!(keys(&json), ["id", "name", "query", "created_by", "created_at", "updated_at"]);
        assert_eq!(keys(&json["query"]), ["kind", "status"], "the query keeps its key order");

        let tagged = object(json!({"tag": "retrieval", "query": "lexical"}));
        let replaced =
            service.save_view("ui-ready", "Open hypotheses", &tagged, "Erin Editor").await.unwrap();
        assert_eq!((replaced.id.as_str(), &replaced.query), (saved.id.as_str(), &tagged));
        assert_eq!(
            (replaced.created_by.as_str(), replaced.created_at),
            ("Olivia Owner", saved.created_at)
        );
        assert!(replaced.updated_at >= saved.updated_at);
        service
            .save_view("ui-ready", "All findings", &object(json!({"kind": "F"})), "Erin Editor")
            .await
            .unwrap();
        let names: Vec<String> = service
            .saved_views("ui-ready")
            .await
            .unwrap()
            .into_iter()
            .map(|view| view.name)
            .collect();
        assert_eq!(names, ["All findings", "Open hypotheses"]);

        let unsupported = service
            .save_view("ui-ready", "x", &object(json!({"sort": "asc"})), "Erin Editor")
            .await
            .unwrap_err();
        assert_eq!(unsupported.message, "Saved view query contains unsupported filters.");
        let large = object(json!({"query": "q".repeat(8180)}));
        let large = service.save_view("ui-ready", "x", &large, "Erin Editor").await.unwrap_err();
        assert_eq!(large.message, "Saved view query is too large.");
        assert!(
            service
                .save_view(
                    "ui-ready",
                    "x",
                    &object(json!({"query": "q".repeat(8179)})),
                    "Erin Editor"
                )
                .await
                .is_ok()
        );
        let unnamed = service.save_view("ui-ready", "\t", &open, "Erin Editor").await.unwrap_err();
        assert_eq!(unnamed.message, "View name cannot be empty.");

        let deleted = service.delete_view("ui-ready", &saved.id, "Erin Editor").await.unwrap();
        assert_eq!(deleted.name, "Open hypotheses");
        let missing = service.delete_view("ui-ready", &saved.id, "Erin Editor").await.unwrap_err();
        assert_eq!(
            (missing.code(), missing.message.as_str()),
            ("not_found", "Saved knowledge view does not exist.")
        );

        let events: Vec<(String, Value)> = event_log(&backend.db, &id)
            .await
            .into_iter()
            .map(|(kind, _, _, payload)| (kind, payload))
            .collect();
        assert_eq!(
            events[0],
            (
                "knowledge.view_saved".to_owned(),
                json!({"view_id": saved.id, "name": "Open hypotheses"})
            )
        );
        assert_eq!(
            events.last().unwrap(),
            &("knowledge.view_removed".to_owned(), json!({"view_id": saved.id}))
        );
    }
}

/// The source part of `test_kickoff_knowledge_sources_runs_and_analytics_are_queryable`.
#[tokio::test]
async fn sources_reject_embedded_credentials() {
    for backend in backends().await {
        let service = backend.service();
        let id = create_project(&backend.db, "ui-ready", Some(&owner())).await;
        let source = |name: &str, source_type: &str, uri: &str| SourceInput {
            name: name.to_owned(),
            source_type: source_type.to_owned(),
            uri: uri.to_owned(),
            ..SourceInput::default()
        };

        let benchmark = SourceInput {
            metadata: object(json!({"purpose": "evaluation", "token_env": "BENCHMARK_TOKEN"})),
            ..source("benchmark", "url", "https://example.test/benchmark")
        };
        let created = service.set_source("ui-ready", &benchmark, "Olivia Owner").await.unwrap();
        assert_eq!((created.source_type.as_str(), created.status.as_str()), ("URL", "ACTIVE"));
        let json = serde_json::to_value(&created).unwrap();
        assert_eq!(
            keys(&json),
            [
                "id",
                "name",
                "type",
                "uri",
                "media_type",
                "metadata",
                "status",
                "created_at",
                "updated_at"
            ]
        );
        assert_eq!(
            json["metadata"],
            json!({"purpose": "evaluation", "token_env": "BENCHMARK_TOKEN"})
        );

        let credential = service
            .set_source(
                "ui-ready",
                &source("unsafe", "URL", "https://example.test/data?access_token=secret"),
                "Olivia Owner",
            )
            .await
            .unwrap_err();
        assert_eq!((credential.code(), credential.http_status()), ("invariant_violation", 422));
        let connector = service
            .set_source(
                "ui-ready",
                &source(
                    "unsafe-connector",
                    "CONNECTOR",
                    "github://user:token@example.test/repository",
                ),
                "Olivia Owner",
            )
            .await
            .unwrap_err();
        assert_eq!(
            connector.message,
            "Source URIs cannot embed credentials; configure a write-only project secret."
        );
        let upload = SourceInput {
            media_type: Some("text/markdown".to_owned()),
            ..source("brief", "UPLOAD", ".limina/sources/abc/brief.md?token=not-a-url")
        };
        let upload = service.set_source("ui-ready", &upload, "Olivia Owner").await.unwrap();
        assert_eq!(
            (upload.source_type.as_str(), upload.media_type.as_deref()),
            ("UPLOAD", Some("text/markdown"))
        );

        for (input, message) in [
            (
                source("x", "ftp", "ftp://example.test"),
                "Source type must be URL, CONNECTOR, or UPLOAD.",
            ),
            (source(" ", "URL", "https://example.test"), "Source name cannot be empty."),
            (source("x", "URL", " "), "Source uri cannot be empty."),
            (
                source(&"n".repeat(201), "URL", "https://example.test"),
                "Source name, URI, or media type exceeds its size limit.",
            ),
            (
                source("x", "URL", "/relative/path"),
                "URL sources must use an absolute HTTP or HTTPS URL.",
            ),
            (
                SourceInput {
                    metadata: object(json!({"nested": [{"Password": "x"}]})),
                    ..source("x", "URL", "https://example.test")
                },
                "Source metadata cannot contain credential values; reference a write-only project secret by environment-variable name.",
            ),
        ] {
            let error = service.set_source("ui-ready", &input, "Olivia Owner").await.unwrap_err();
            assert_eq!(error.message, message);
        }

        let names: Vec<String> = service
            .sources("ui-ready")
            .await
            .unwrap()
            .into_iter()
            .map(|source| source.name)
            .collect();
        assert_eq!(names, ["benchmark", "brief"]);

        let replaced = service
            .set_source(
                "ui-ready",
                &source("benchmark", "CONNECTOR", "drive://benchmarks/v2"),
                "Erin Editor",
            )
            .await
            .unwrap();
        assert_eq!(
            (replaced.id.as_str(), replaced.source_type.as_str(), replaced.created_at),
            (created.id.as_str(), "CONNECTOR", created.created_at)
        );
        assert!(replaced.metadata.is_empty());

        let removed = service.remove_source("ui-ready", &created.id, "Erin Editor").await.unwrap();
        assert_eq!(
            (removed.status.as_str(), removed.uri.as_str()),
            ("REMOVED", "drive://benchmarks/v2")
        );
        assert!(removed.updated_at >= replaced.updated_at);
        assert_eq!(service.sources("ui-ready").await.unwrap().len(), 1);
        let again =
            service.remove_source("ui-ready", &created.id, "Erin Editor").await.unwrap_err();
        assert_eq!(
            (again.code(), again.message.as_str()),
            ("not_found", "Active project source does not exist.")
        );
        let restored = service.set_source("ui-ready", &benchmark, "Erin Editor").await.unwrap();
        assert_eq!(
            (restored.id.as_str(), restored.status.as_str()),
            (created.id.as_str(), "ACTIVE")
        );

        let events: Vec<(String, Value)> = event_log(&backend.db, &id)
            .await
            .into_iter()
            .map(|(kind, _, _, payload)| (kind, payload))
            .collect();
        assert_eq!(
            events[0],
            (
                "source.configured".to_owned(),
                json!({"source_id": created.id, "name": "benchmark", "type": "URL"})
            )
        );
        assert!(events.contains(&(
            "source.removed".to_owned(),
            json!({"source_id": created.id, "name": "benchmark"})
        )));
    }
}

/// Rows written by the Python runtime (with its JSON text and timestamp formats) read back the
/// same way.
#[tokio::test]
async fn rows_written_by_the_python_runtime_read_back() {
    let backend = backends().await.into_iter().next().unwrap();
    let service: CollaborationService = backend.service();
    let id = create_project(&backend.db, "legacy", Some(&owner())).await;
    let mut tx = backend.db.begin_write().await.unwrap();
    tx.execute_script(&format!(
        "INSERT INTO saved_knowledge_views (id, challenge_id, name, \"query\", created_by, created_at, updated_at) \
         VALUES ('view-1', '{id}', 'Legacy', '{{\"kind\": \"H\", \"query\": \"caf\\u00e9\"}}', 'Olivia Owner', \
         '2026-10-01 09:30:00.000000', '2026-10-01 09:30:00.123456');"
    ))
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let views = serde_json::to_value(service.saved_views("legacy").await.unwrap()).unwrap();
    assert_eq!(
        views,
        json!([{"id": "view-1", "name": "Legacy", "query": {"kind": "H", "query": "café"}, "created_by": "Olivia Owner",
                "created_at": "2026-10-01T09:30:00+00:00", "updated_at": "2026-10-01T09:30:00.123456+00:00"}])
    );
}
