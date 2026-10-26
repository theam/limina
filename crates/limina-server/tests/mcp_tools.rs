//! One scenario per tool area (projects, steering, knowledge, sources, members, runs and
//! analytics) through the SDK client, plus who a write is recorded for and how failures reach
//! the caller.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod app_support;
mod mcp_support;

use app_support::*;
use limina_domain::research::{NewExperiment, NewHypothesis};
use limina_server::mcp::tools::TOOLS;
use mcp_support::*;
use rmcp::model::{ErrorCode, ResourceContents};
use rmcp::service::ServiceError;
use serde_json::{Value, json};

async fn token_server() -> McpServer {
    serve(runtime(ScriptedSession::always(Reply::ok(complete()))).await, token_auth()).await
}

async fn people_server() -> McpServer {
    serve(runtime(ScriptedSession::always(Reply::ok(complete()))).await, people_auth()).await
}

fn new_project(slug: &str) -> Value {
    json!({
        "slug": slug,
        "name": "Retrieval quality",
        "mission": "Find why retrieval regressed.",
        "success_criteria": "The regression is explained by reproducible evidence.",
    })
}

/// A hypothesis and a running experiment with one observation; returns their IDs.
async fn seed_knowledge(server: &McpServer, slug: &str) -> (String, String) {
    let service = server.test.runtime.service();
    let hypothesis = NewHypothesis {
        title: "Stale index".into(),
        statement: "The index was built before the schema change.".into(),
        mechanism: "Old embeddings".into(),
        generalization: "Any rebuild skew".into(),
        shortcut_risks: "None".into(),
        test_plan: "Rebuild and compare".into(),
    };
    let h = service.create_hypothesis(slug, hypothesis, "Limina", &key()).await.unwrap();
    let experiment = NewExperiment {
        hypothesis_id: h.id.clone(),
        title: "Rebuild the index".into(),
        objective: "Compare recall".into(),
        procedure: "Rebuild, rerun the eval".into(),
        success_criteria: "Recall recovers".into(),
        guardrails: "Same eval set".into(),
    };
    let e = service.create_experiment(slug, experiment, "Limina", &key()).await.unwrap();
    service.claim_experiment(slug, &e.id, 600, "limina:worker", &key()).await.unwrap();
    service
        .append_observation(slug, &e.id, "Recall 0.71", None, "limina:worker", &key())
        .await
        .unwrap();
    (h.id, e.id)
}

#[tokio::test]
async fn projects_are_created_listed_inspected_and_managed() {
    let server = token_server().await;
    let client = server.client(Some("maya")).await;
    let created = structured(&client, "limina_create_project", new_project("retrieval")).await;
    assert_eq!(created["slug"], "retrieval");
    assert_eq!(created["runtime"], "codex");
    assert_eq!(created["status"], "CREATED");
    assert_eq!(created["role"], "OWNER");

    let listed = structured(&client, "limina_list_projects", json!({})).await;
    assert_eq!(listed["total"], 1);
    assert_eq!(listed["items"][0]["slug"], "retrieval");

    let status =
        structured(&client, "limina_get_project_status", json!({"project": "retrieval"})).await;
    assert_eq!(status["project"]["mission"], "Find why retrieval regressed.");
    assert_eq!(status["pending_guidance"], 0);

    let archive = json!({"project": "retrieval", "action": "archive", "idempotency_key": "a-1"});
    let archived = structured(&client, "limina_manage_project", archive.clone()).await;
    assert_eq!(archived["status"], "ARCHIVED");
    assert_eq!(structured(&client, "limina_manage_project", archive).await, archived);
    assert_eq!(structured(&client, "limina_list_projects", json!({})).await["total"], 0);
    let all = structured(&client, "limina_list_projects", json!({"include_archived": true})).await;
    assert_eq!(all["items"][0]["status"], "ARCHIVED");

    let refused = tool_error(
        &client,
        "limina_manage_project",
        json!({"project": "retrieval", "action": "start"}),
    )
    .await;
    assert!(refused.starts_with("Error executing tool limina_manage_project: "), "{refused}");
    client.cancel().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn steering_is_durable_guidance() {
    let server = token_server().await;
    let client = server.client(Some("maya")).await;
    structured(&client, "limina_create_project", new_project("steered")).await;
    let steer =
        json!({"project": "steered", "message": "Check the tokenizer first.", "kind": "ANSWER"});
    let receipt = structured(&client, "limina_steer_project", steer).await;
    assert_eq!(receipt["delivery"], "QUEUED");
    assert_eq!(receipt["kind"], "ANSWER");
    assert_eq!(receipt["status"], "PENDING");

    let guidance = structured(&client, "limina_list_guidance", json!({"project": "steered"})).await;
    assert_eq!(guidance["total"], 1);
    assert_eq!(guidance["items"][0]["body"], "Check the tokenizer first.");
    assert_eq!(guidance["items"][0]["actor"], "maya");
    assert_eq!(guidance["items"][0]["id"], receipt["id"]);
    let pending = json!({"project": "steered", "status": "ACKNOWLEDGED"});
    assert_eq!(structured(&client, "limina_list_guidance", pending).await["total"], 0);

    let activity = structured(&client, "limina_list_activity", json!({"project": "steered"})).await;
    let types: Vec<&str> = activity["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event["type"].as_str())
        .collect();
    assert_eq!(types, ["project.created", "guidance.received"]);
    let cursor = activity["cursor"].as_i64().unwrap();
    let after = json!({"project": "steered", "after": cursor});
    let later = structured(&client, "limina_list_activity", after).await;
    assert_eq!(later["events"], json!([]));
    assert_eq!(later["cursor"], cursor);
    client.cancel().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn knowledge_is_queried_read_and_reviewed() {
    let server = token_server().await;
    let client = server.client(Some("maya")).await;
    structured(&client, "limina_create_project", new_project("kb")).await;
    let (h, e) = seed_knowledge(&server, "kb").await;

    let page =
        structured(&client, "limina_query_knowledge", json!({"project": "kb", "kind": "E"})).await;
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["id"], e);
    // Listed artifacts carry no observations at all (Python's dicts), not `null`.
    assert!(page["items"][0].get("observations").is_none());
    let search = json!({"project": "kb", "query": "index", "limit": 1});
    let found = structured(&client, "limina_query_knowledge", search).await;
    assert_eq!(found["total"], 2);
    assert_eq!(found["items"].as_array().unwrap().len(), 1);
    assert!(found["next_cursor"].is_string());

    let experiment =
        structured(&client, "limina_get_knowledge", json!({"project": "kb", "artifact_id": e}))
            .await;
    assert_eq!(experiment["observations"][0]["body"], "Recall 0.71");
    let hypothesis =
        structured(&client, "limina_get_knowledge", json!({"project": "kb", "artifact_id": h}))
            .await;
    assert!(hypothesis.get("observations").is_none());
    let keys: Vec<&str> = hypothesis.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys.last(), Some(&"tags"));

    let graph = structured(&client, "limina_get_knowledge_graph", json!({"project": "kb"})).await;
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 2);
    assert!(
        graph["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| edge["source_id"] == h.as_str() && edge["target_id"] == e.as_str())
    );

    let review = structured(&client, "limina_review_project", json!({"project": "kb"})).await;
    assert_eq!(review["hypotheses"][0]["id"], h);
    assert_eq!(review["experiments"][0]["id"], e);
    assert_eq!(review["active_work"][0]["id"], e);
    assert_eq!(review["knowledge_total"], 2);
    assert!(
        review["recent_activity"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["actor"] != "limina:worker")
    );

    let artifact = read(&client, &format!("limina://projects/kb/knowledge/{e}")).await.unwrap();
    let ResourceContents::TextResourceContents { text, .. } = &artifact.contents[0] else {
        panic!()
    };
    assert_eq!(serde_json::from_str::<Value>(text).unwrap(), experiment);
    let snapshot = read(&client, "limina://projects/kb/snapshot").await.unwrap();
    let ResourceContents::TextResourceContents { text, .. } = &snapshot.contents[0] else {
        panic!()
    };
    let files = serde_json::from_str::<Value>(text).unwrap();
    assert!(files["files"].as_object().unwrap().keys().any(|path| path.contains(&h)));
    client.cancel().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn resources_hold_variables_and_redacted_secrets() {
    let server = token_server().await;
    let client = server.client(Some("maya")).await;
    structured(&client, "limina_create_project", new_project("vars")).await;
    let set = json!({"project": "vars", "name": "EVAL_SET", "value": "s3://bucket/eval.parquet"});
    let variable = structured(&client, "limina_set_project_variable", set).await;
    assert_eq!(variable["value"], "s3://bucket/eval.parquet");
    server
        .test
        .ops()
        .set_secret("vars", "API_KEY", "sk-test-not-real", &key(), &local("maya"))
        .await
        .unwrap();

    let result = call(&client, "limina_list_project_resources", json!({"project": "vars"})).await;
    let listed = result.structured_content.clone().unwrap();
    let items = listed["result"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    // One text block per item, as FastMCP rendered lists.
    assert_eq!(texts(&result).len(), 2);
    let secret = items.iter().find(|item| item["type"] == "SECRET").unwrap();
    assert_eq!(secret["configured"], true);
    assert!(secret.get("value").is_none());
    assert!(!serde_json::to_string(&result).unwrap().contains("sk-test-not-real"));

    let removed = structured(
        &client,
        "limina_remove_project_resource",
        json!({"project": "vars", "name": "EVAL_SET"}),
    )
    .await;
    assert_eq!(removed["status"], "REMOVED");
    assert_eq!(removed["value"], Value::Null);
    let removed_secret = structured(
        &client,
        "limina_remove_project_resource",
        json!({"project": "vars", "name": "API_KEY"}),
    )
    .await;
    assert_eq!(removed_secret["configured"], false);
    client.cancel().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn secrets_are_never_tool_arguments() {
    for tool in TOOLS {
        for param in tool.params {
            assert!(
                !param.name.contains("secret") && !param.name.contains("token"),
                "{}",
                tool.name
            );
        }
    }
    let server = token_server().await;
    let client = server.client(Some("maya")).await;
    let refused = tool_error(
        &client,
        "limina_set_project_secret",
        json!({"project": "p", "name": "K", "value": "v"}),
    )
    .await;
    assert_eq!(refused, "Unknown tool: limina_set_project_secret");
    client.cancel().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn sources_are_registered_for_the_runtime() {
    let server = token_server().await;
    let client = server.client(Some("maya")).await;
    structured(&client, "limina_create_project", new_project("sourced")).await;
    let register = json!({
        "project": "sourced",
        "name": "Eval notes",
        "source_type": "URL",
        "uri": "https://example.test/notes",
        "media_type": "text/html",
    });
    let source = structured(&client, "limina_register_project_source", register).await;
    assert_eq!(source["type"], "URL");
    assert_eq!(source["media_type"], "text/html");
    assert_eq!(source["metadata"], json!({}));
    let sources = server.test.ops().sources("sourced", &local("maya")).await.unwrap();
    assert_eq!(serde_json::to_value(&sources).unwrap(), json!([source]));
    let upload = json!({"project": "sourced", "name": "x", "source_type": "UPLOAD", "uri": "x"});
    let refused = tool_error(&client, "limina_register_project_source", upload).await;
    assert!(
        refused.contains("Input should be 'URL' or 'CONNECTOR' [type=literal_error]"),
        "{refused}"
    );
    client.cancel().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn members_are_listed_and_managed_by_owners() {
    let server = people_server().await;
    let owner = server.client_as("owner").await;
    structured(&owner, "limina_create_project", new_project("team")).await;
    let add = json!({
        "project": "team",
        "subject": "viewer",
        "display_name": "Victor Viewer",
        "role": "VIEWER",
        "email": "viewer@example.test",
    });
    let member = structured(&owner, "limina_set_project_member", add).await;
    assert_eq!(member["role"], "VIEWER");
    assert_eq!(member["email"], "viewer@example.test");
    let members = call(&owner, "limina_list_project_members", json!({"project": "team"})).await;
    let listed = members.structured_content.clone().unwrap();
    let subjects: Vec<&str> =
        listed["result"].as_array().unwrap().iter().filter_map(|m| m["subject"].as_str()).collect();
    assert_eq!(subjects.len(), 2);
    assert!(subjects.contains(&"owner") && subjects.contains(&"viewer"));

    // Roles are the operations' rules: viewers read, but neither steer nor manage members.
    let viewer = server.client_as("viewer").await;
    let status = structured(&viewer, "limina_get_project_status", json!({"project": "team"})).await;
    assert_eq!(status["project"]["role"], "VIEWER");
    let steer =
        tool_error(&viewer, "limina_steer_project", json!({"project": "team", "message": "go"}))
            .await;
    assert!(steer.starts_with("Error executing tool limina_steer_project: "), "{steer}");
    let promote =
        json!({"project": "team", "subject": "viewer", "display_name": "V", "role": "OWNER"});
    let promote = tool_error(&viewer, "limina_set_project_member", promote).await;
    assert!(promote.starts_with("Error executing tool limina_set_project_member: "), "{promote}");
    // Projects the caller is not a member of stay invisible.
    let editor = server.client_as("editor").await;
    assert_eq!(structured(&editor, "limina_list_projects", json!({})).await["total"], 0);
    owner.cancel().await.unwrap();
    viewer.cancel().await.unwrap();
    editor.cancel().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn runs_and_analytics_report_managed_turns() {
    let server = token_server().await;
    let client = server.client(Some("maya")).await;
    structured(&client, "limina_create_project", new_project("running")).await;
    let started = structured(
        &client,
        "limina_manage_project",
        json!({"project": "running", "action": "start"}),
    )
    .await;
    assert_eq!(started["status"], "RUNNING");
    wait_until("the run to complete", || async {
        let runs = server.test.ops().runs("running", None, None, 50, &local("maya")).await.unwrap();
        runs.items.first().is_some_and(|run| run.status == "COMPLETED")
    })
    .await;

    let runs = structured(&client, "limina_list_runtime_runs", json!({"project": "running"})).await;
    assert_eq!(runs["total"], 1);
    assert_eq!(runs["items"][0]["status"], "COMPLETED");
    assert_eq!(runs["items"][0]["summary"], "Checkpoint complete.");
    let failed = json!({"project": "running", "status": "FAILED"});
    assert_eq!(structured(&client, "limina_list_runtime_runs", failed).await["total"], 0);

    let analytics = structured(
        &client,
        "limina_get_project_analytics",
        json!({"project": "running", "days": 7}),
    )
    .await;
    assert_eq!(analytics["window"]["days"], 7);
    assert_eq!(analytics["runs"]["total"], 1);
    assert_eq!(analytics["runs"]["by_status"], json!({"COMPLETED": 1}));
    client.cancel().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn writes_are_recorded_for_the_right_person() {
    let server = people_server().await;
    // An identity-provider principal acts as itself whatever `actor` says.
    let owner = server.client_as("owner").await;
    let mut create = new_project("audited");
    create["actor"] = json!("someone-else");
    structured(&owner, "limina_create_project", create).await;
    // A principal without a display name is recorded as the MCP client.
    server
        .test
        .ops()
        .set_member(
            "audited",
            &limina_domain::collaboration::MemberChange {
                subject: "service-account".into(),
                display_name: "Service".into(),
                email: None,
                role: "EDITOR".into(),
            },
            &owner_principal(),
        )
        .await
        .unwrap();
    let service = server.client_as("service").await;
    structured(
        &service,
        "limina_steer_project",
        json!({"project": "audited", "message": "nightly"}),
    )
    .await;
    let events = server
        .test
        .ops()
        .activity("audited", 0, None, 50, &owner_principal())
        .await
        .unwrap()
        .events;
    let actors: Vec<&str> = events.iter().map(|event| event.actor.as_str()).collect();
    assert_eq!(actors, ["Olivia Owner", "Olivia Owner", "mcp:client"]);

    // Static-token callers name the teammate with `actor` (stripped); otherwise X-Limina-Actor.
    let tokens = token_server().await;
    let client = tokens.client(Some("maya")).await;
    let mut create = new_project("named");
    create["actor"] = json!("  sam  ");
    structured(&client, "limina_create_project", create).await;
    structured(&client, "limina_steer_project", json!({"project": "named", "message": "go"})).await;
    let events =
        tokens.test.ops().activity("named", 0, None, 50, &local("maya")).await.unwrap().events;
    let actors: Vec<&str> = events.iter().map(|event| event.actor.as_str()).collect();
    assert_eq!(actors, ["sam", "maya"]);
    // Idempotency keys are scoped to the person, as over REST.
    let steer = json!({"project": "named", "message": "once", "idempotency_key": "k-1"});
    let first = structured(&client, "limina_steer_project", steer.clone()).await;
    let mut as_sam = steer.clone();
    as_sam["actor"] = json!("sam");
    assert_ne!(structured(&client, "limina_steer_project", as_sam).await["id"], first["id"]);
    assert_eq!(structured(&client, "limina_steer_project", steer).await["id"], first["id"]);
    for client in [owner, service, client] {
        client.cancel().await.unwrap();
    }
    server.close().await;
    tokens.close().await;
}

fn owner_principal() -> limina_core::Principal {
    app_support::owner()
}

#[tokio::test]
async fn failures_reach_the_caller_like_python() {
    let server = token_server().await;
    let client = server.client(Some("maya")).await;
    let missing =
        tool_error(&client, "limina_get_project_status", json!({"project": "nope"})).await;
    assert_eq!(
        missing,
        "Error executing tool limina_get_project_status: Challenge 'nope' does not exist."
    );
    let invalid = tool_error(
        &client,
        "limina_list_activity",
        json!({"project": "p", "limit": "many", "after": -1}),
    )
    .await;
    assert_eq!(
        invalid,
        "Error executing tool limina_list_activity: 2 validation errors for \
         limina_list_activityArguments\nafter\n  Input should be greater than or equal to 0 \
         [type=greater_than_equal]\nlimit\n  Input should be a valid integer, unable to parse \
         string as an integer [type=int_parsing]"
    );
    let long = tool_error(&client, "limina_create_project", json!({
        "slug": "x", "name": "x", "mission": "x", "success_criteria": "x", "actor": "a".repeat(201),
    }))
    .await;
    assert!(
        long.ends_with("String should have at most 200 characters [type=string_too_long]"),
        "{long}"
    );
    let bad_flag =
        tool_error(&client, "limina_list_projects", json!({"include_archived": "maybe"})).await;
    assert!(bad_flag.ends_with("unable to interpret input [type=bool_parsing]"), "{bad_flag}");

    // Resources fail with JSON-RPC errors carrying the API's code and details.
    let Err(ServiceError::McpError(error)) = read(&client, "limina://projects/nope/status").await
    else {
        panic!("the read succeeded");
    };
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert_eq!(error.message, "Challenge 'nope' does not exist.");
    assert_eq!(error.data.unwrap()["code"], "not_found");
    let Err(ServiceError::McpError(error)) = read(&client, "limina://projects/nope").await else {
        panic!("the read succeeded");
    };
    assert_eq!(error.code, ErrorCode::RESOURCE_NOT_FOUND);
    assert_eq!(error.message, "Unknown resource: limina://projects/nope");
    client.cancel().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn mcp_supports_focused_evidence_queries_and_durable_kpi_definitions() {
    let server = token_server().await;
    let client = server.client(Some("maya")).await;
    structured(&client, "limina_create_project", new_project("measured")).await;
    let (h, e) = seed_knowledge(&server, "measured").await;
    let children = structured(
        &client,
        "limina_query_knowledge",
        json!({"project":"measured","parent":h,"limit":1}),
    )
    .await;
    assert_eq!(children["items"][0]["id"], e);
    let exact =
        structured(&client, "limina_query_knowledge", json!({"project":"measured","query":"h001"}))
            .await;
    assert_eq!(exact["total"], 1);
    let input = json!({"project":"measured","metric_id":"recall","metric":{"name":"Recall","unit":"%","direction":"at_least","target":95,"baseline":70},"expected_version":0,"idempotency_key":"define-recall"});
    let metric = structured(&client, "limina_set_project_metric", input.clone()).await;
    assert_eq!(metric["version"], 1);
    assert!(metric["current"].is_null());
    assert_eq!(structured(&client, "limina_set_project_metric", input).await, metric);
    let listed =
        structured(&client, "limina_list_project_metrics", json!({"project":"measured"})).await;
    assert_eq!(listed["result"][0], metric);
    let incomplete=tool_error(&client,"limina_set_project_metric",json!({"project":"measured","metric_id":"recall","metric":{"name":"Recall","direction":"at_least","target":95,"current":85,"evidence_id":e},"expected_version":1})).await;
    assert!(incomplete.contains("completed experiment"));
    client.cancel().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn workspace_history_and_published_outputs_are_readable_from_the_local_agent() {
    let server = token_server().await;
    let client = server.client(Some("maya")).await;
    structured(&client, "limina_create_project", new_project("workspace-reads")).await;
    let (h, _) = seed_knowledge(&server, "workspace-reads").await;
    let empty =
        structured(&client, "limina_get_project_workspace", json!({"project":"workspace-reads"}))
            .await;
    assert!(empty["assessment"].is_null());
    let versions = structured(
        &client,
        "limina_get_knowledge_revisions",
        json!({"project":"workspace-reads", "artifact_id":h}),
    )
    .await;
    assert!(!versions["items"].as_array().unwrap().is_empty());
    assert!(versions["next_before"].is_null());
    let directory = server.test.root.join("workspaces/workspace-reads/outputs");
    tokio::fs::create_dir_all(&directory).await.unwrap();
    tokio::fs::write(directory.join("report.md"), "# A published report").await.unwrap();
    let files =
        structured(&client, "limina_list_project_outputs", json!({"project":"workspace-reads"}))
            .await;
    assert_eq!(files["items"][0]["name"], "report.md");
    let preview = structured(
        &client,
        "limina_read_project_output",
        json!({"project":"workspace-reads", "name":"report.md"}),
    )
    .await;
    assert_eq!(preview["text"], "# A published report");
    let escaped = tool_error(
        &client,
        "limina_read_project_output",
        json!({"project":"workspace-reads", "name":"../secret"}),
    )
    .await;
    assert!(escaped.contains("Output not found"));
    client.cancel().await.unwrap();
    server.close().await;
}
