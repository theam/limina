//! The HTTP contract against the Python API, from frozen fixtures the Python runtime generated
//! (`fixtures/http_*.json`, `fixtures/app_shapes.json`):
//!
//! - every recorded request (validation errors, domain errors, unknown routes and methods,
//!   authentication) gets the same status and the same body;
//! - the OpenAPI document has the same paths, operations, parameters, request bodies, responses
//!   and component schemas;
//! - the console's main endpoints, replayed over HTTP for a scripted session, answer with the
//!   Python bodies' shapes (and the same values where they are deterministic), and every body
//!   matches its documented schema.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod app_support;
mod http_support;

use std::collections::{BTreeMap, BTreeSet};

use app_support::*;
use axum::http::Method;
use http_support::*;
use limina_engines::RuntimeEvent;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

// ─── Recorded requests ────────────────────────────────────────────────────────

fn multipart(name: Option<&str>, file: &Value) -> (String, Vec<u8>) {
    let boundary = "limina-test-boundary";
    let mut body = String::new();
    if let Some(name) = name {
        body.push_str(&format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\n{name}\r\n"
        ));
    }
    let (filename, content, media_type) =
        (file[0].as_str().unwrap(), file[1].as_str().unwrap(), file[2].as_str().unwrap());
    body.push_str(&format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\
         Content-Type: {media_type}\r\n\r\n{content}\r\n--{boundary}--\r\n"
    ));
    (format!("multipart/form-data; boundary={boundary}"), body.into_bytes())
}

async fn replay(api: &Api, request: &Value) -> Answer {
    let method = Method::from_bytes(request["method"].as_str().unwrap().as_bytes()).unwrap();
    let path = request["path"].as_str().unwrap();
    let headers: Vec<(String, String)> = request["headers"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, value)| (name.to_lowercase(), value.as_str().unwrap().to_owned()))
        .collect();
    let headers = h(&headers);
    let body = &request["body"];
    match request["kind"].as_str().unwrap() {
        "json" => api.json(method, path, &headers, body).await,
        "raw" => {
            let (content_type, text) = (body[0].as_str().unwrap(), body[1].as_str().unwrap());
            api.raw(method, path, &headers, content_type, text.as_bytes().to_vec()).await
        }
        "form" => {
            let form = url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(
                    body.as_object().unwrap().iter().map(|(k, v)| (k, v.as_str().unwrap())),
                )
                .finish();
            let content_type = "application/x-www-form-urlencoded";
            api.raw(method, path, &headers, content_type, form.into_bytes()).await
        }
        "files" => {
            let (content_type, bytes) = multipart(body[0].as_str(), &body[1]);
            api.raw(method, path, &headers, &content_type, bytes).await
        }
        _ => api.call(method, path, &headers).await,
    }
}

/// JSON syntax errors name the problem in Python's words only approximately (see
/// `http::call`); everything else must match exactly.
fn comparable(label: &str, mut body: Value) -> Value {
    if label.starts_with("invalid JSON") {
        body["error"]["details"]["errors"][0]["ctx"] = json!("<python json message>");
    }
    body
}

#[tokio::test]
async fn recorded_requests_get_the_python_answers() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/http_errors.json")).unwrap();
    let api =
        Api::new(runtime(ScriptedSession::always(Reply::ok(complete()))).await, local_tokens())
            .await;
    let create =
        json!({"slug": "demo", "name": "Demo", "objective": "Obj", "success_criteria": "Done"});
    let headers = [("authorization", "Bearer secret"), ("idempotency-key", "fixed-key")];
    api.post("/v1/projects", &headers, &create).await.expect(201);
    for case in &cases {
        let label = case["label"].as_str().unwrap();
        let request = &case["request"];
        if request.get("after_failures").is_some() {
            for _ in 0..10 {
                replay(&api, request).await;
            }
        }
        let reply = replay(&api, request).await;
        assert_eq!(
            reply.status.as_u16(),
            case["status"].as_u64().unwrap() as u16,
            "{label}: {}",
            reply.text()
        );
        let actual = comparable(label, reply.json());
        let expected = comparable(label, case["response"].clone());
        assert_eq!(actual, expected, "{label}");
        // Starlette listed the first partially matching route's methods; axum lists every method
        // the path serves.
        if let Some(allow) = case["allow"].as_str() {
            let served = reply.header("allow").unwrap_or_default();
            assert!(allow.split(", ").all(|method| served.contains(method)), "{label}: {served}");
        }
        if label == "rate limited" {
            assert_eq!(reply.header("retry-after"), Some("60"));
        }
    }
    api.close().await;
}

// ─── OpenAPI ──────────────────────────────────────────────────────────────────

/// A schema reduced to what both generators must agree on: references, types, nullability,
/// enumerations, items, map values and the length and range constraints.
fn shape(schema: &Value) -> BTreeSet<String> {
    let Some(object) = schema.as_object() else { return BTreeSet::from(["any".to_owned()]) };
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        return BTreeSet::from([reference.rsplit('/').next().unwrap().to_owned()]);
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(members) = object.get(key).and_then(Value::as_array) {
            return members.iter().flat_map(shape).collect();
        }
    }
    let types: Vec<String> = match object.get("type") {
        Some(Value::String(kind)) => vec![kind.clone()],
        Some(Value::Array(kinds)) => {
            kinds.iter().map(|kind| kind.as_str().unwrap().to_owned()).collect()
        }
        _ => Vec::new(),
    };
    let enumeration: Option<Vec<Value>> = object
        .get("enum")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| object.get("const").map(|value| vec![value.clone()]));
    if types.is_empty() && enumeration.is_none() {
        return BTreeSet::from(["any".to_owned()]);
    }
    let mut members = BTreeSet::new();
    if let Some(values) = enumeration {
        let mut named: Vec<String> =
            values.iter().filter(|value| !value.is_null()).map(Value::to_string).collect();
        named.sort();
        members.insert(format!("enum{named:?}"));
        if values.iter().any(Value::is_null) || types.iter().any(|kind| kind == "null") {
            members.insert("null".to_owned());
        }
        return members;
    }
    let constraints: BTreeMap<&str, &Value> = ["minLength", "maxLength", "minimum", "maximum"]
        .into_iter()
        .filter_map(|key| object.get(key).map(|value| (key, value)))
        .collect();
    for kind in types {
        let described = match kind.as_str() {
            "null" => "null".to_owned(),
            "array" => format!("list{:?}", shape(object.get("items").unwrap_or(&Value::Null))),
            "object" => match object.get("additionalProperties") {
                Some(Value::Bool(true)) | Some(Value::Object(_))
                    if object.get("properties").is_none() =>
                {
                    let values = object.get("additionalProperties").unwrap();
                    format!(
                        "map{:?}",
                        if values == &json!(true) {
                            BTreeSet::from(["any".to_owned()])
                        } else {
                            shape(values)
                        }
                    )
                }
                _ => "object".to_owned(),
            },
            "integer" => format!("integer{constraints:?}"),
            other => format!("{other}{constraints:?}"),
        };
        members.insert(described);
    }
    members
}

fn component_summary(schema: &Value) -> Value {
    let properties = schema["properties"].as_object().cloned().unwrap_or_default();
    let mut required: Vec<String> = schema["required"]
        .as_array()
        .map(|items| items.iter().map(|item| item.as_str().unwrap().to_owned()).collect())
        .unwrap_or_default();
    required.sort();
    json!({
        // The fixture was written with sorted keys, so only the set of properties is compared.
        "properties": properties.keys().collect::<BTreeSet<_>>(),
        "required": required,
        "shapes": properties.iter().map(|(name, value)| (name.clone(), json!(format!("{:?}", shape(value))))).collect::<Map<_, _>>(),
        "defaults": properties.iter().filter_map(|(name, value)| value.get("default").map(|default| (name.clone(), default.clone()))).collect::<Map<_, _>>(),
    })
}

fn operation_summary(operation: &Value) -> Value {
    let parameters: Vec<Value> = operation["parameters"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|parameter| {
            json!({
                "name": parameter["name"],
                "in": parameter["in"],
                "required": parameter["required"].as_bool().unwrap_or(false),
                "shape": format!("{:?}", shape(&parameter["schema"])),
                "default": parameter["schema"].get("default"),
            })
        })
        .collect();
    let content = |value: &Value| -> Value {
        value["content"]
            .as_object()
            .map(|content| {
                content
                    .iter()
                    .map(|(media, body)| {
                        (media.clone(), json!(format!("{:?}", shape(&body["schema"]))))
                    })
                    .collect::<Map<_, _>>()
            })
            .map_or(Value::Null, Value::Object)
    };
    let responses: Map<String, Value> = operation["responses"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(status, response)| (status.clone(), content(response)))
        .collect();
    json!({
        "operationId": operation["operationId"],
        "summary": operation["summary"],
        "tags": operation.get("tags"),
        "security": operation.get("security"),
        "parameters": parameters,
        "requestBody": operation.get("requestBody").map(|body| json!({"required": body["required"], "content": content(body)})),
        "responses": responses,
    })
}

/// Operations the Rust API added after the Python contract was recorded.
const ADDED: &[&str] = &[
    "get /v1/projects/{slug}/workspace",
    "get /v1/projects/{slug}/knowledge/{artifact_id}/history",
    "get /v1/projects/{slug}/outputs",
    "get /v1/projects/{slug}/outputs/{name}",
    "get /v1/projects/{slug}/outputs/{name}/download",
    "get /v1/projects/{slug}/knowledge/{artifact_id}/document",
    "get /v1/projects/{slug}/metrics",
    "put /v1/projects/{slug}/metrics/{metric_id}",
    "get /v1/runtime/engines/{engine}/models",
    "get /v1/connections",
    "post /v1/connections",
    "get /v1/connections/{id}",
    "patch /v1/connections/{id}",
    "delete /v1/connections/{id}",
    "post /v1/connections/{id}/disconnect",
    "post /v1/connections/{id}/connect",
    "get /v1/projects/{slug}/connections",
    "put /v1/projects/{slug}/connections/{id}",
    "delete /v1/projects/{slug}/connections/{id}",
    "post /v1/projects/{slug}/engine-accounts/claude-code/login",
    "delete /v1/projects/{slug}/engine-accounts/claude-code/login/{login_id}",
    "get /v1/projects/{slug}/engine-accounts/claude-code/login/{login_id}",
    "put /v1/projects/{slug}/engine-accounts/claude-code/login/{login_id}",
    "get /v1/projects/{slug}/sharing",
    "put /v1/projects/{slug}/sharing",
    "delete /v1/projects/{slug}/sharing",
    "get /v1/me/shared-projects",
];

#[tokio::test]
async fn the_openapi_document_matches_the_python_one() {
    let python: Value = serde_json::from_str(include_str!("fixtures/http_openapi.json")).unwrap();
    let api =
        Api::new(runtime(ScriptedSession::always(Reply::ok(complete()))).await, local_tokens())
            .await;
    let rust = api.get("/openapi.json", &[]).await.expect(200).json();
    assert_eq!(rust["openapi"], "3.1.0");
    assert_eq!(rust["info"]["title"], python["info"]["title"]);
    assert_eq!(rust["info"]["description"], python["info"]["description"]);
    assert_eq!(rust["components"]["securitySchemes"], python["components"]["securitySchemes"]);

    let paths = |document: &Value| -> Vec<(String, Vec<String>)> {
        document["paths"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(path, item)| (path.clone(), item.as_object().unwrap().keys().cloned().collect()))
            .collect()
    };
    let mut expected_paths = paths(&python);
    let mut actual_paths = paths(&rust);
    expected_paths
        .iter_mut()
        .chain(actual_paths.iter_mut())
        .for_each(|(_, methods)| methods.sort());
    expected_paths.sort();
    actual_paths.sort();
    // The Python contract is kept whole; the Rust API may add operations (listed in ADDED).
    let missing: Vec<String> = expected_paths
        .iter()
        .flat_map(|(path, methods)| methods.iter().map(move |method| format!("{method} {path}")))
        .filter(|operation| {
            let (method, path) = operation.split_once(' ').unwrap();
            !actual_paths.iter().any(|(p, ms)| p == path && ms.iter().any(|m| m == method))
        })
        .collect();
    assert!(missing.is_empty(), "operations missing from the Rust API: {missing:?}");
    let added: BTreeSet<String> = actual_paths
        .iter()
        .flat_map(|(path, methods)| methods.iter().map(move |method| format!("{method} {path}")))
        .filter(|operation| {
            let (method, path) = operation.split_once(' ').unwrap();
            !expected_paths.iter().any(|(p, ms)| p == path && ms.iter().any(|m| m == method))
        })
        .collect();
    let allowed: BTreeSet<String> = ADDED.iter().map(|operation| (*operation).to_owned()).collect();
    assert_eq!(added, allowed, "new operations must be listed in ADDED");
    let mut differences = Vec::new();
    for (path, methods) in &expected_paths {
        for method in methods {
            let expected = operation_summary(&python["paths"][path][method]);
            let mut operation = rust["paths"][path][method].clone();
            if path == "/v1/projects/{slug}/knowledge" && method == "get" {
                let parameters = operation["parameters"].as_array_mut().unwrap();
                assert!(parameters.iter().any(
                    |parameter| parameter["name"] == "parent" && parameter["required"] != true
                ));
                parameters.retain(|parameter| parameter["name"] != "parent");
            }
            let actual = operation_summary(&operation);
            if actual != expected {
                differences
                    .push(format!("{method} {path}\n  rust:   {actual}\n  python: {expected}"));
            }
        }
    }

    let names = |document: &Value| -> BTreeSet<String> {
        document["components"]["schemas"].as_object().unwrap().keys().cloned().collect()
    };
    let missing_schemas: Vec<String> = names(&python).difference(&names(&rust)).cloned().collect();
    assert!(missing_schemas.is_empty(), "schemas missing from the Rust API: {missing_schemas:?}");
    for name in names(&python) {
        let expected = component_summary(&python["components"]["schemas"][&name]);
        let mut schema = rust["components"]["schemas"][&name].clone();
        if ["CreateProjectRequest", "UpdateProjectRequest", "ProjectResponse"]
            .contains(&name.as_str())
        {
            for field in ["model", "reasoning_effort"] {
                assert!(schema["properties"][field].is_object(), "missing project default {field}");
                assert!(
                    !schema["required"]
                        .as_array()
                        .is_some_and(|items| items.contains(&json!(field))),
                    "project defaults must stay optional"
                );
                schema["properties"].as_object_mut().unwrap().remove(field);
            }
        }
        let actual = component_summary(&schema);
        if actual != expected {
            differences.push(format!("schema {name}\n  rust:   {actual}\n  python: {expected}"));
        }
    }
    assert!(
        differences.is_empty(),
        "{} differences:\n{}",
        differences.len(),
        differences.join("\n")
    );
    api.close().await;
}

// ─── The console's endpoints for a scripted session ───────────────────────────

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

/// Deserializes a body into its documented schema type (unknown fields are denied).
fn documented<T: DeserializeOwned>(name: &str, value: &Value) {
    if let Err(error) = serde_json::from_value::<T>(value.clone()) {
        panic!("{name} does not match its schema: {error}\n{value:#}");
    }
}

#[tokio::test]
async fn console_endpoints_answer_like_the_python_api() {
    use limina_server::http::schemas as s;

    let python: Map<String, Value> =
        serde_json::from_str(include_str!("fixtures/app_shapes.json")).unwrap();
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
    let principals = std::collections::HashMap::from([("owner".to_owned(), owner())]);
    let api = Api::new(runtime(agent).await, token_auth(principals)).await;
    let a = auth("owner");
    let a = h(&a);
    let c = || command("owner", None, "spoofed");
    let mut rust = Map::new();
    let mut record = |name: &str, reply: Answer| {
        rust.insert(
            name.to_owned(),
            json!({"status": reply.status.as_u16(), "body": reply.json()}),
        );
    };

    record(
        "create",
        api.post(
            "/v1/projects",
            &h(&c()),
            &json!({"slug": "p", "name": "P", "objective": "M", "success_criteria": "S"}),
        )
        .await,
    );
    record("list", api.get("/v1/projects", &a).await);
    record("get", api.get("/v1/projects/p", &a).await);
    record("status", api.get("/v1/projects/p/status", &a).await);
    record("patch", api.patch("/v1/projects/p", &a, &json!({"context": "ctx"})).await);
    record("templates", api.get("/v1/project-templates", &a).await);
    record("preflight", api.get("/v1/projects/p/preflight", &a).await);
    record("engines", api.get("/v1/runtime/engines", &a).await);
    record("engine_accounts", api.get("/v1/projects/p/engine-accounts", &a).await);
    record(
        "var",
        api.put("/v1/projects/p/resources/variables/V", &h(&c()), &json!({"value": "x"})).await,
    );
    record(
        "secret",
        api.put("/v1/projects/p/resources/secrets/S", &h(&c()), &json!({"value": "y"})).await,
    );
    record("resources", api.get("/v1/projects/p/resources", &a).await);
    record("members", api.get("/v1/projects/p/members", &a).await);
    record(
        "set_member",
        api.put(
            "/v1/projects/p/members",
            &a,
            &json!({"subject": "v", "display_name": "V", "role": "VIEWER"}),
        )
        .await,
    );
    record("ticket", api.call(Method::POST, "/v1/projects/p/live-ticket", &a).await);
    record(
        "source",
        api.put(
            "/v1/projects/p/sources",
            &a,
            &json!({"name": "b", "type": "URL", "uri": "https://example.test/x"}),
        )
        .await,
    );
    record("sources", api.get("/v1/projects/p/sources", &a).await);
    let service = api.test.runtime.service();
    let h1 = service
        .create_hypothesis(
            "p",
            limina_domain::research::NewHypothesis {
                title: "H".into(),
                statement: "s".into(),
                mechanism: "m".into(),
                generalization: "g".into(),
                shortcut_risks: "r".into(),
                test_plan: "t".into(),
            },
            "Limina",
            &key(),
        )
        .await
        .unwrap();
    let e1 = service
        .create_experiment(
            "p",
            limina_domain::research::NewExperiment {
                hypothesis_id: h1.id.clone(),
                title: "E".into(),
                objective: "o".into(),
                procedure: "p".into(),
                success_criteria: "s".into(),
                guardrails: "g".into(),
            },
            "Limina",
            &key(),
        )
        .await
        .unwrap();
    service.claim_experiment("p", &e1.id, 600, "limina:x", &key()).await.unwrap();
    service.append_observation("p", &e1.id, "obs", None, "limina:x", &key()).await.unwrap();
    record("knowledge", api.get("/v1/projects/p/knowledge", &a).await);
    record("knowledge_h", api.get(&format!("/v1/projects/p/knowledge/{}", h1.id), &a).await);
    record("knowledge_e", api.get(&format!("/v1/projects/p/knowledge/{}", e1.id), &a).await);
    record("graph", api.get("/v1/projects/p/knowledge/graph", &a).await);
    record(
        "tag",
        api.call(Method::PUT, &format!("/v1/projects/p/knowledge/{}/tags/x", h1.id), &a).await,
    );
    record(
        "comment",
        api.post(
            &format!("/v1/projects/p/knowledge/{}/comments", h1.id),
            &h(&c()),
            &json!({"body": "b"}),
        )
        .await,
    );
    record(
        "revisions",
        api.get(&format!("/v1/projects/p/knowledge/{}/revisions", h1.id), &a).await,
    );
    record(
        "view",
        api.put(
            "/v1/projects/p/knowledge/views",
            &a,
            &json!({"name": "n", "query": {"kind": "H"}}),
        )
        .await,
    );
    record("status2", api.get("/v1/projects/p/status", &a).await);
    record("review", api.get("/v1/projects/p/review", &a).await);
    record("steer", api.post("/v1/projects/p/steering", &h(&c()), &json!({"body": "go"})).await);
    record("guidance", api.get("/v1/projects/p/guidance", &a).await);
    record("start", api.call(Method::POST, "/v1/projects/p/actions/start", &h(&c())).await);
    api.test.runtime.supervisor().join("p").await;
    let runs = api.get("/v1/projects/p/runs", &a).await;
    let run_id = runs.json()["items"][0]["id"].as_str().unwrap().to_owned();
    record("runs", runs);
    record("run", api.get(&format!("/v1/projects/p/runs/{run_id}"), &a).await);
    record("analytics", api.get("/v1/projects/p/analytics", &a).await);
    record("events", api.get("/v1/projects/p/events", &a).await);
    record("snapshot", api.get("/v1/projects/p/snapshot", &a).await);
    record("remove", api.delete("/v1/projects/p/resources/V", &h(&c())).await);
    record("err", api.get("/v1/projects/zz", &a).await);

    for (name, expected) in &python {
        let actual = rust.get(name).unwrap_or_else(|| panic!("{name} was not replayed"));
        assert_eq!(actual["status"], expected["status"], "{name}: {:#}", actual["body"]);
        assert_eq!(
            skeleton(&actual["body"]),
            skeleton(&expected["body"]),
            "{name}: {:#}",
            actual["body"]
        );
    }
    for name in ["templates", "preflight", "engines", "engine_accounts", "tag", "err"] {
        assert_eq!(rust[name]["body"], python[name]["body"], "{name}");
    }

    // Every body is the documented shape.
    let body = |name: &str| rust[name]["body"].clone();
    for name in ["create", "get", "patch", "start"] {
        documented::<s::ProjectResponse>(name, &body(name));
    }
    documented::<s::ProjectPage>("list", &body("list"));
    documented::<s::ProjectStatusResponse>("status", &body("status"));
    documented::<s::ProjectStatusResponse>("status2", &body("status2"));
    documented::<Vec<s::KickoffTemplate>>("templates", &body("templates"));
    documented::<s::PreflightResponse>("preflight", &body("preflight"));
    documented::<Vec<s::EngineAvailability>>("engines", &body("engines"));
    documented::<Vec<s::ProjectEngineAccountResponse>>("engine_accounts", &body("engine_accounts"));
    for name in ["var", "secret", "remove"] {
        documented::<s::ResourceResponse>(name, &body(name));
    }
    documented::<Vec<s::ResourceResponse>>("resources", &body("resources"));
    documented::<Vec<s::MemberResponse>>("members", &body("members"));
    documented::<s::MemberResponse>("set_member", &body("set_member"));
    documented::<s::LiveTicketResponse>("ticket", &body("ticket"));
    documented::<s::SourceResponse>("source", &body("source"));
    documented::<Vec<s::SourceResponse>>("sources", &body("sources"));
    documented::<s::KnowledgePage>("knowledge", &body("knowledge"));
    documented::<s::ArtifactResponse>("knowledge_h", &body("knowledge_h"));
    documented::<s::ArtifactResponse>("knowledge_e", &body("knowledge_e"));
    documented::<s::KnowledgeGraphResponse>("graph", &body("graph"));
    documented::<s::TagResponse>("tag", &body("tag"));
    documented::<s::CommentResponse>("comment", &body("comment"));
    documented::<Vec<s::RevisionResponse>>("revisions", &body("revisions"));
    documented::<s::SavedViewResponse>("view", &body("view"));
    documented::<s::ReviewResponse>("review", &body("review"));
    documented::<s::GuidanceReceipt>("steer", &body("steer"));
    documented::<s::GuidancePage>("guidance", &body("guidance"));
    documented::<s::RuntimeRunPage>("runs", &body("runs"));
    documented::<s::RuntimeRunDetail>("run", &body("run"));
    documented::<s::AnalyticsResponse>("analytics", &body("analytics"));
    documented::<s::EventPage>("events", &body("events"));
    documented::<s::SnapshotResponse>("snapshot", &body("snapshot"));
    documented::<s::ErrorResponse>("err", &body("err"));
    documented::<s::MeResponse>("me", &api.get("/v1/me", &a).await.json());
    documented::<s::HealthResponse>("healthz", &api.get("/healthz", &a).await.json());
    api.close().await;
}
