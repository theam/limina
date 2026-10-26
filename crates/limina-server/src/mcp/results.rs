//! Tool results in the shapes the Python MCP server returned.
//!
//! Python's tools returned the operations' dicts, not the HTTP response models. The Rust
//! operations return the HTTP shapes, which differ in three places; these projections undo that:
//!
//! - **Artifacts** carry `tags` last and `observations` only when the artifact was read on its
//!   own and is an experiment (the HTTP model adds `"observations": null` everywhere).
//! - **Resources** list their own fields first and then `value` (variables) or `configured`
//!   (secrets), always present even when `null` (the HTTP model drops `null`s, and the review
//!   page carries both fields).
//! - **Reviews and statuses** contain those artifacts and resources.
//!
//! Everything else (projects, pages, guidance, runs, analytics, members, sources, preflight,
//! activity) already serializes like the Python dicts and passes through unchanged.

use limina_core::{LiminaError, Result, internal};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::app::{
    ArtifactResponse, KnowledgeGraphResponse, KnowledgePageResponse, ProjectStatusResponse,
    ResourceResponse, ReviewResource, ReviewResponse,
};

/// Any serializable result as JSON.
pub fn json<T: Serialize>(value: &T) -> Result<Value> {
    serde_json::to_value(value).map_err(internal)
}

/// The keys of `public_artifact`/`_artifact_dict`, in Python's order (`observations` follows
/// when present).
const ARTIFACT_KEYS: [&str; 10] = [
    "id",
    "kind",
    "title",
    "status",
    "content",
    "hypothesis_id",
    "experiment_id",
    "created_at",
    "updated_at",
    "tags",
];

/// The keys every resource has, in `public_resource`'s order.
const RESOURCE_KEYS: [&str; 5] = ["name", "type", "status", "created_at", "updated_at"];

pub fn artifact(artifact: &ArtifactResponse) -> Result<Value> {
    let full = object(json(artifact)?)?;
    let mut shaped = pick(&full, &ARTIFACT_KEYS);
    if let Some(observations) = full.get("observations").filter(|value| !value.is_null()) {
        shaped.insert("observations".to_owned(), observations.clone());
    }
    Ok(Value::Object(shaped))
}

fn artifacts(items: &[ArtifactResponse]) -> Result<Value> {
    items.iter().map(artifact).collect::<Result<Vec<_>>>().map(Value::Array)
}

/// `public_resource`: variables carry `value`, every other type `configured`.
pub fn resource(resource: &ResourceResponse) -> Result<Value> {
    shaped_resource(&resource.resource_type, resource.value.as_deref(), resource.configured, || {
        json(resource)
    })
}

pub fn resources(items: &[ResourceResponse]) -> Result<Vec<Value>> {
    items.iter().map(resource).collect()
}

fn review_resource(resource: &ReviewResource) -> Result<Value> {
    shaped_resource(&resource.resource_type, resource.value.as_deref(), resource.configured, || {
        json(resource)
    })
}

fn shaped_resource(
    resource_type: &str,
    value: Option<&str>,
    configured: Option<bool>,
    full: impl FnOnce() -> Result<Value>,
) -> Result<Value> {
    let mut shaped = pick(&object(full()?)?, &RESOURCE_KEYS);
    if resource_type == "VARIABLE" {
        shaped.insert("value".to_owned(), value.map_or(Value::Null, |value| value.into()));
    } else {
        shaped.insert("configured".to_owned(), configured.map_or(Value::Null, Value::Bool));
    }
    Ok(Value::Object(shaped))
}

/// `public_status`.
pub fn status(status: &ProjectStatusResponse) -> Result<Value> {
    Ok(Value::Object(status_fields(status)?))
}

fn status_fields(status: &ProjectStatusResponse) -> Result<Map<String, Value>> {
    let mut shaped = Map::new();
    shaped.insert("project".to_owned(), json(&status.project)?);
    shaped.insert("knowledge".to_owned(), json(&status.knowledge)?);
    shaped.insert("active_work".to_owned(), artifacts(&status.active_work)?);
    shaped.insert("pending_guidance".to_owned(), status.pending_guidance.into());
    shaped.insert("event_cursor".to_owned(), status.event_cursor.into());
    Ok(shaped)
}

/// `review`: the status fields, then resources, knowledge by kind and recent activity.
pub fn review(review: &ReviewResponse) -> Result<Value> {
    let mut shaped = status_fields(&review.status)?;
    let resources: Vec<Value> =
        review.resources.iter().map(review_resource).collect::<Result<_>>()?;
    shaped.insert("resources".to_owned(), Value::Array(resources));
    shaped.insert("hypotheses".to_owned(), artifacts(&review.hypotheses)?);
    shaped.insert("experiments".to_owned(), artifacts(&review.experiments)?);
    shaped.insert("findings".to_owned(), artifacts(&review.findings)?);
    shaped.insert("recent_activity".to_owned(), json(&review.recent_activity)?);
    shaped.insert("knowledge_cursor".to_owned(), json(&review.knowledge_cursor)?);
    shaped.insert("knowledge_total".to_owned(), review.knowledge_total.into());
    Ok(Value::Object(shaped))
}

/// `query_knowledge`.
pub fn knowledge_page(page: &KnowledgePageResponse) -> Result<Value> {
    let mut shaped = Map::new();
    shaped.insert("items".to_owned(), artifacts(&page.items)?);
    shaped.insert("next_cursor".to_owned(), json(&page.next_cursor)?);
    shaped.insert("total".to_owned(), page.total.into());
    shaped.insert("search_backend".to_owned(), page.search_backend.clone().into());
    Ok(Value::Object(shaped))
}

/// `knowledge_graph`.
pub fn knowledge_graph(graph: &KnowledgeGraphResponse) -> Result<Value> {
    let mut shaped = Map::new();
    shaped.insert("nodes".to_owned(), artifacts(&graph.nodes)?);
    shaped.insert("edges".to_owned(), json(&graph.edges)?);
    Ok(Value::Object(shaped))
}

/// The JSON text Python put next to structured results and in resources
/// (`json.dumps(value, indent=2, ensure_ascii=False)`).
pub fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn object(value: Value) -> Result<Map<String, Value>> {
    match value {
        Value::Object(map) => Ok(map),
        other => Err(LiminaError::transport(format!("Expected a JSON object, got {other}."))),
    }
}

fn pick(source: &Map<String, Value>, keys: &[&str]) -> Map<String, Value> {
    keys.iter()
        .map(|key| ((*key).to_owned(), source.get(*key).cloned().unwrap_or(Value::Null)))
        .collect()
}
