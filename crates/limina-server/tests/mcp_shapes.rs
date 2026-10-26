//! The Rust MCP server against the Python one (`fixtures/mcp_shapes.json`, a frozen recording):
//! the same server instructions, tools (names, descriptions, input
//! and output schemas, annotations) and resources, and for one scripted session the same
//! results, call by call.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod app_support;
mod mcp_support;

use std::time::Duration;

use app_support::*;
use limina_domain::research::{NewExperiment, NewHypothesis};
use limina_engines::RuntimeEvent;
use mcp_support::*;
use rmcp::model::{ErrorCode, ErrorData, ResourceContents};
use rmcp::service::ServiceError;
use serde_json::{Map, Value, json};

fn fixture() -> Map<String, Value> {
    serde_json::from_str(include_str!("fixtures/mcp_shapes.json")).unwrap()
}

/// The JSON's structure: object keys in order, value kinds, array lengths.
fn skeleton(value: &Value) -> Value {
    match value {
        Value::Null => json!("null"),
        Value::Bool(_) => json!("bool"),
        Value::Number(number) if number.is_f64() => json!("float"),
        Value::Number(_) => json!("int"),
        Value::String(_) => json!("string"),
        Value::Array(items) => Value::Array(items.iter().map(skeleton).collect()),
        Value::Object(map) => Value::Object(
            map.iter().map(|(key, item)| (key.clone(), skeleton(item))).collect::<Map<_, _>>(),
        ),
    }
}

/// The JSON with run-specific values (ids, timestamps, durations) replaced by placeholders.
fn stable(value: &Value) -> Value {
    const TIMINGS: [&str; 4] = [
        "duration_ms",
        "average_duration_ms",
        "p95_duration_ms",
        "average_acknowledgement_seconds",
    ];
    match value {
        Value::String(text) => {
            let bytes = text.as_bytes();
            let uuid = text.len() == 36 && [8, 13, 18, 23].iter().all(|at| bytes[*at] == b'-');
            let time = text.len() >= 20 && bytes[4] == b'-' && bytes[10] == b'T';
            let date = text.len() == 10 && bytes[4] == b'-' && bytes[7] == b'-';
            if uuid {
                json!("<uuid>")
            } else if time {
                json!("<time>")
            } else if date {
                json!("<date>")
            } else {
                value.clone()
            }
        }
        Value::Array(items) => Value::Array(items.iter().map(stable).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| {
                    let item =
                        if TIMINGS.contains(&key.as_str()) { json!("<n>") } else { stable(item) };
                    (key.clone(), item)
                })
                .collect(),
        ),
        _ => value.clone(),
    }
}

/// Pydantic's report without the echoed input and the documentation link, which the Rust
/// server leaves out.
fn without_pydantic_extras(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("For further information visit"))
        .map(|line| match line.find("[type=") {
            Some(at) => {
                let rest = &line[at + 6..];
                let end = rest.find([',', ']']).unwrap_or(rest.len());
                format!("{}[type={}]", &line[..at], &rest[..end])
            }
            None => line.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn the_server_publishes_what_python_published() {
    let python = fixture();
    let server =
        serve(runtime(ScriptedSession::always(Reply::ok(complete()))).await, token_auth()).await;
    let client = server.client(Some("maya")).await;

    let info = client.peer_info().unwrap();
    let expected = &python["initialize"];
    assert_eq!(info.instructions.as_deref(), expected["instructions"].as_str());
    let server_info = info.server_info.as_ref().unwrap();
    assert_eq!(server_info.name, expected["serverInfo"]["name"].as_str().unwrap());
    assert_eq!(server_info.version, env!("CARGO_PKG_VERSION"));
    let capabilities = serde_json::to_value(&info.capabilities).unwrap();
    for capability in ["prompts", "resources", "tools"] {
        assert!(capabilities.get(capability).is_some(), "{capability} is not advertised");
    }

    // Same tools, in the same order, serialized identically (including key order).
    let tools = client.list_all_tools().await.unwrap();
    let rust: Vec<Value> = tools.iter().map(|tool| serde_json::to_value(tool).unwrap()).collect();
    let expected = python["tools"].as_array().unwrap();
    assert_eq!(rust.len(), expected.len() + 6);
    assert_eq!(rust[expected.len()]["name"], "limina_list_project_metrics");
    assert_eq!(rust[expected.len() + 1]["name"], "limina_set_project_metric");
    let metric = &rust[expected.len() + 1]["inputSchema"]["properties"]["metric"];
    assert_eq!(metric["type"], "object");
    assert_eq!(metric["properties"]["current"]["type"], json!(["number", "null"]));
    assert_eq!(metric["properties"]["direction"]["enum"], json!(["at_least", "at_most"]));
    for field in ["name", "direction", "target"] {
        assert!(metric["required"].as_array().unwrap().contains(&json!(field)));
    }
    for (rust, python) in rust.iter().zip(expected) {
        let mut rust = rust.clone();
        if rust["name"] == "limina_query_knowledge" {
            assert_eq!(rust["inputSchema"]["properties"]["parent"]["type"], "string");
            assert_eq!(rust["inputSchema"]["properties"]["parent"]["default"], "");
            rust["inputSchema"]["properties"].as_object_mut().unwrap().shift_remove("parent");
        }
        if rust["name"] == "limina_create_project" {
            // Additive optional defaults; all pre-existing tool fields keep the frozen contract.
            for field in ["model", "reasoning_effort"] {
                assert_eq!(rust["inputSchema"]["properties"][field]["type"], "string");
                rust["inputSchema"]["properties"].as_object_mut().unwrap().shift_remove(field);
            }
        }
        assert_eq!(&rust, python, "tool {}", python["name"]);
        assert_eq!(serde_json::to_string(&rust).unwrap(), serde_json::to_string(python).unwrap());
    }

    let resources = client.list_all_resources().await.unwrap();
    assert_eq!(serde_json::to_value(&resources).unwrap(), python["resources"]);
    let templates = client.list_all_resource_templates().await.unwrap();
    assert_eq!(serde_json::to_value(&templates).unwrap(), python["resource_templates"]);
    let prompts = client.list_all_prompts().await.unwrap();
    assert!(prompts.is_empty());

    client.cancel().await.unwrap();
    server.close().await;
}

/// What one step of the scripted session produced.
enum Outcome {
    Tool { is_error: bool, structured: Option<Value>, texts: Vec<String> },
    Read(Result<TextResource, ErrorData>),
}

/// A resource read's one text content.
struct TextResource {
    uri: String,
    mime_type: Option<String>,
    text: String,
}

#[tokio::test]
async fn results_match_the_python_server_call_by_call() {
    let python = fixture();
    let calls = python["calls"].as_object().unwrap();
    let mut done = complete();
    done.decision.summary = "Done.".into();
    done.decision.current_objective = "O".into();
    done.decision.next_step = "N".into();
    done.continuation_id = Some("c".into());
    done.turn_id = "t".into();
    done.usage = Some(limina_domain::usage::RuntimeUsage {
        input_tokens: Some(1),
        output_tokens: Some(2),
        ..Default::default()
    });
    let agent =
        ScriptedSession::always(Reply::ok(done).continuation("c").event(RuntimeEvent::new(
            "runtime.codex",
            "hello",
            detail(json!({"kind": "message", "text": "hi"})),
        )));
    let server = serve(runtime(agent).await, token_auth()).await;
    let client = server.client(Some("maya")).await;
    let maya = local("maya");
    let ops = server.test.ops();
    let service = server.test.runtime.service();

    let mut outcomes: Vec<(String, Outcome)> = Vec::new();
    let mut ids = (String::new(), String::new());
    for (name, step) in calls {
        // The steps outside MCP, at the same points as the Python script.
        match name.as_str() {
            "resources" => {
                ops.set_secret("p", "S", "y", &key(), &maya).await.unwrap();
            }
            "status_with_work" => {
                let hypothesis = NewHypothesis {
                    title: "H".into(),
                    statement: "s".into(),
                    mechanism: "m".into(),
                    generalization: "g".into(),
                    shortcut_risks: "r".into(),
                    test_plan: "t".into(),
                };
                let h = service.create_hypothesis("p", hypothesis, "Limina", &key()).await.unwrap();
                let experiment = NewExperiment {
                    hypothesis_id: h.id.clone(),
                    title: "E".into(),
                    objective: "o".into(),
                    procedure: "p".into(),
                    success_criteria: "s".into(),
                    guardrails: "g".into(),
                };
                let e = service.create_experiment("p", experiment, "Limina", &key()).await.unwrap();
                service.claim_experiment("p", &e.id, 600, "limina:x", &key()).await.unwrap();
                service
                    .append_observation("p", &e.id, "obs", None, "limina:x", &key())
                    .await
                    .unwrap();
                ids = (h.id, e.id);
            }
            "runs" => {
                wait_until("the run to complete", || async {
                    let runs = ops.runs("p", None, None, 50, &maya).await.unwrap();
                    runs.items.first().is_some_and(|run| run.status == "COMPLETED")
                })
                .await;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            _ => {}
        }
        let outcome = if let Some(tool) = step["tool"].as_str() {
            let mut arguments = step["arguments"].clone();
            // Artifact IDs are the ones this run created (the same IDs as in Python).
            if let Some(id) = arguments.get("artifact_id").and_then(Value::as_str) {
                assert!(id == ids.0 || id == ids.1 || id == "H404", "unexpected artifact {id}");
            }
            if let Some(object) = arguments.as_object_mut() {
                object.retain(|_, value| !value.is_null());
            }
            let result = call(&client, tool, arguments).await;
            Outcome::Tool {
                is_error: result.is_error == Some(true),
                structured: result.structured_content.clone(),
                texts: texts(&result),
            }
        } else {
            let uri = step["uri"].as_str().unwrap();
            Outcome::Read(match read(&client, uri).await {
                Ok(result) => match &result.contents[0] {
                    ResourceContents::TextResourceContents { uri, mime_type, text, .. } => {
                        Ok(TextResource {
                            uri: uri.clone(),
                            mime_type: mime_type.clone(),
                            text: text.clone(),
                        })
                    }
                    other => panic!("not text: {other:?}"),
                },
                Err(ServiceError::McpError(error)) => Err(error),
                Err(other) => panic!("{name}: {other}"),
            })
        };
        outcomes.push((name.clone(), outcome));
    }

    for (name, outcome) in outcomes {
        let python = &calls[&name];
        match outcome {
            Outcome::Tool { is_error, structured, texts } => {
                assert_eq!(is_error, python["is_error"].as_bool().unwrap(), "{name}: {texts:?}");
                let python_texts: Vec<&str> =
                    python["texts"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
                if is_error {
                    assert_eq!(texts.len(), 1, "{name}");
                    assert_eq!(texts[0], without_pydantic_extras(python_texts[0]), "{name}");
                    continue;
                }
                let structured = structured.unwrap();
                assert_eq!(skeleton(&structured), skeleton(&python["structured"]), "{name}");
                assert_eq!(stable(&structured), stable(&python["structured"]), "{name}");
                assert_eq!(texts.len(), python_texts.len(), "{name}");
                for (rust, python) in texts.iter().zip(python_texts) {
                    let rust: Value = serde_json::from_str(rust).unwrap();
                    let python: Value = serde_json::from_str(python).unwrap();
                    assert_eq!(skeleton(&rust), skeleton(&python), "{name} text");
                }
                let items = match structured.get("result") {
                    Some(Value::Array(items)) if python["structured"].get("result").is_some() => {
                        items.clone()
                    }
                    _ => vec![structured.clone()],
                };
                let rendered: Vec<String> =
                    items.iter().map(|item| serde_json::to_string_pretty(item).unwrap()).collect();
                assert_eq!(texts, rendered, "{name}: the text is the structured content");
            }
            Outcome::Read(Ok(TextResource { uri, mime_type, text })) => {
                let contents = &python["result"]["contents"][0];
                assert_eq!(uri, contents["uri"].as_str().unwrap(), "{name}");
                assert_eq!(mime_type.as_deref(), contents["mimeType"].as_str(), "{name}");
                let rust: Value = serde_json::from_str(&text).unwrap();
                let python: Value =
                    serde_json::from_str(contents["text"].as_str().unwrap()).unwrap();
                assert_eq!(skeleton(&rust), skeleton(&python), "{name}");
                if name != "read_snapshot" {
                    assert_eq!(stable(&rust), stable(&python), "{name}");
                }
            }
            Outcome::Read(Err(ErrorData { code, message, data, .. })) => {
                // Python sent code 0 and an SDK-wrapped message; see the module docs.
                let python_message = python["error"]["message"].as_str().unwrap();
                match name.as_str() {
                    "read_missing" => {
                        assert_eq!(code, ErrorCode::INVALID_PARAMS);
                        assert!(python_message.ends_with(message.as_ref()), "{python_message}");
                        assert_eq!(
                            data,
                            Some(json!({"code": "not_found", "details": {"challenge": "missing"}}))
                        );
                    }
                    "read_unknown" => {
                        assert_eq!(code, ErrorCode::RESOURCE_NOT_FOUND);
                        assert_eq!(message, python_message);
                    }
                    other => panic!("{other} failed: {message}"),
                }
            }
        }
    }
    client.cancel().await.unwrap();
    server.close().await;
}
