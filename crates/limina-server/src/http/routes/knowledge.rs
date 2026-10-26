//! Knowledge: search, the graph, relations, saved views, revisions, comments and tags.

use axum::extract::{Request, State};

use super::{Params, created, ok, parts};
use crate::app::PUBLIC_COMMAND_ID_LIMIT;
use crate::http::App;
use crate::http::call::{Call, Checked, json_body};
use crate::http::error::ApiResult;
use crate::http::requests::{CommentRequest, RelationRequest, SavedViewRequest};
use crate::http::schemas::{
    ActorHeader, ArtifactResponse, CommandHeader, CommentResponse, KnowledgeGraphResponse,
    KnowledgePage, KnowledgeParams, PublicErrors, RelationResponse, RevisionResponse,
    SavedViewResponse, TagResponse,
};
use crate::http::validate::{Int, Text};
use limina_domain::knowledge::KnowledgeQuery;

/// Searches and filters knowledge, most recently updated first.
#[utoipa::path(get, path = "/v1/projects/{slug}/knowledge",
    operation_id = "query_knowledge_v1_projects__slug__knowledge_get", summary = "Query Knowledge",
    params(("slug" = String, Path), KnowledgeParams, ActorHeader),
    responses((status = 200, description = "Successful Response", body = KnowledgePage), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn query(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    let text = call.query_text("query", Text::max(500));
    let kind = call.query_text("kind", Text::ANY);
    let status = call.query_text("status", Text::ANY);
    let tag = call.query_text("tag", Text::ANY);
    let parent = call.query_text("parent", Text::max(200));
    let cursor = call.query_text("cursor", Text::ANY);
    let limit = call.query_int("limit", 50, Int::between(1, 200));
    call.check()?;
    let query = KnowledgeQuery {
        query: text.checked()?,
        kind: kind.checked()?,
        status: status.checked()?,
        tag: tag.checked()?,
        parent: parent.checked()?,
        cursor: cursor.checked()?,
        limit: limit.checked()?,
    };
    ok(app.runtime.operations().query_knowledge(&slug, &query, &principal.checked()?).await?)
}

/// Read one complete note without exporting the project's entire research graph.
#[utoipa::path(get, path = "/v1/projects/{slug}/knowledge/{artifact_id}/document",
    operation_id = "knowledge_document", summary = "Read Knowledge Document",
    params(("slug" = String, Path), ("artifact_id" = String, Path), ActorHeader),
    responses((status = 200, body = crate::http::schemas::SnapshotResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn document(
    State(app): State<App>,
    Params((slug, key)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().knowledge_document(&slug, &key, &principal.checked()?).await?)
}

/// Every artifact and every edge between them.
#[utoipa::path(get, path = "/v1/projects/{slug}/knowledge/graph",
    operation_id = "knowledge_graph_v1_projects__slug__knowledge_graph_get", summary = "Knowledge Graph",
    params(("slug" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = KnowledgeGraphResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn graph(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().knowledge_graph(&slug, &principal.checked()?).await?)
}

/// Links two artifacts (editors).
#[utoipa::path(post, path = "/v1/projects/{slug}/knowledge/relations",
    operation_id = "create_relation_v1_projects__slug__knowledge_relations_post", summary = "Create Relation",
    params(("slug" = String, Path), ActorHeader), request_body = RelationRequest,
    responses((status = 201, description = "Successful Response", body = RelationResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn create_relation(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let (parts, body) = request.into_parts();
    let body = json_body(&parts, body, app.settings.json_body_limit_bytes).await?;
    let mut call = Call::new(parts);
    let principal = call.caller().await?;
    let relation = RelationRequest::parse(&body, call.errors());
    call.check()?;
    let operations = app.runtime.operations();
    created(operations.create_relation(&slug, &relation.checked()?, &principal.checked()?).await?)
}

#[utoipa::path(delete, path = "/v1/projects/{slug}/knowledge/relations/{relation_id}",
    operation_id = "delete_relation_v1_projects__slug__knowledge_relations__relation_id__delete",
    summary = "Delete Relation", params(("slug" = String, Path), ("relation_id" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = RelationResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn delete_relation(
    State(app): State<App>,
    Params((slug, relation_id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    let operations = app.runtime.operations();
    ok(operations.delete_relation(&slug, &relation_id, &principal.checked()?).await?)
}

#[utoipa::path(get, path = "/v1/projects/{slug}/knowledge/views",
    operation_id = "saved_views_v1_projects__slug__knowledge_views_get", summary = "Saved Views",
    params(("slug" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = Vec<SavedViewResponse>), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn saved_views(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().saved_views(&slug, &principal.checked()?).await?)
}

/// Saves a named knowledge query (editors); saving an existing name replaces it.
#[utoipa::path(put, path = "/v1/projects/{slug}/knowledge/views",
    operation_id = "save_view_v1_projects__slug__knowledge_views_put", summary = "Save View",
    params(("slug" = String, Path), ActorHeader), request_body = SavedViewRequest,
    responses((status = 200, description = "Successful Response", body = SavedViewResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn save_view(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let (parts, body) = request.into_parts();
    let body = json_body(&parts, body, app.settings.json_body_limit_bytes).await?;
    let mut call = Call::new(parts);
    let principal = call.caller().await?;
    let view = SavedViewRequest::parse(&body, call.errors());
    call.check()?;
    let view = view.checked()?;
    let operations = app.runtime.operations();
    ok(operations.save_view(&slug, &view.name, &view.query, &principal.checked()?).await?)
}

#[utoipa::path(delete, path = "/v1/projects/{slug}/knowledge/views/{view_id}",
    operation_id = "delete_view_v1_projects__slug__knowledge_views__view_id__delete", summary = "Delete View",
    params(("slug" = String, Path), ("view_id" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = SavedViewResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn delete_view(
    State(app): State<App>,
    Params((slug, view_id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().delete_view(&slug, &view_id, &principal.checked()?).await?)
}

/// One artifact with its tags (and an experiment's observations).
#[utoipa::path(get, path = "/v1/projects/{slug}/knowledge/{artifact_id}",
    operation_id = "get_knowledge_v1_projects__slug__knowledge__artifact_id__get", summary = "Get Knowledge",
    params(("slug" = String, Path), ("artifact_id" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = ArtifactResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn get(
    State(app): State<App>,
    Params((slug, artifact_id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    artifact(app, slug, artifact_id, request).await
}

/// `GET …/knowledge/relations`: the static `relations` route only takes `POST`, so this is the
/// artifact route with `relations` as its ID (as the Python router matched it).
pub async fn get_relations_artifact(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    artifact(app, slug, "relations".to_owned(), request).await
}

async fn artifact(app: App, slug: String, artifact_id: String, request: Request) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().get_knowledge(&slug, &artifact_id, &principal.checked()?).await?)
}

#[utoipa::path(get, path = "/v1/projects/{slug}/knowledge/{artifact_id}/revisions",
    operation_id = "knowledge_revisions_v1_projects__slug__knowledge__artifact_id__revisions_get",
    summary = "Knowledge Revisions", params(("slug" = String, Path), ("artifact_id" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = Vec<RevisionResponse>), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn revisions(
    State(app): State<App>,
    Params((slug, artifact_id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().revisions(&slug, &artifact_id, &principal.checked()?).await?)
}

#[utoipa::path(get, path = "/v1/projects/{slug}/knowledge/{artifact_id}/comments",
    operation_id = "knowledge_comments_v1_projects__slug__knowledge__artifact_id__comments_get",
    summary = "Knowledge Comments", params(("slug" = String, Path), ("artifact_id" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = Vec<CommentResponse>), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn comments(
    State(app): State<App>,
    Params((slug, artifact_id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().comments(&slug, &artifact_id, &principal.checked()?).await?)
}

/// Comments on an artifact (editors), idempotent on the caller's key.
#[utoipa::path(post, path = "/v1/projects/{slug}/knowledge/{artifact_id}/comments",
    operation_id = "add_knowledge_comment_v1_projects__slug__knowledge__artifact_id__comments_post",
    summary = "Add Knowledge Comment",
    params(("slug" = String, Path), ("artifact_id" = String, Path), CommandHeader, ActorHeader),
    request_body = CommentRequest,
    responses((status = 201, description = "Successful Response", body = CommentResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn add_comment(
    State(app): State<App>,
    Params((slug, artifact_id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let (parts, body) = request.into_parts();
    let body = json_body(&parts, body, app.settings.json_body_limit_bytes).await?;
    let mut call = Call::new(parts);
    let command_id = call.command_id(PUBLIC_COMMAND_ID_LIMIT);
    let principal = call.caller().await?;
    let comment = CommentRequest::parse(&body, call.errors());
    call.check()?;
    let comment = app
        .runtime
        .operations()
        .add_comment(
            &slug,
            &artifact_id,
            &comment.checked()?.body,
            &command_id.checked()?,
            &principal.checked()?,
        )
        .await?;
    created(comment)
}

#[utoipa::path(get, path = "/v1/projects/{slug}/knowledge/{artifact_id}/tags",
    operation_id = "knowledge_tags_v1_projects__slug__knowledge__artifact_id__tags_get", summary = "Knowledge Tags",
    params(("slug" = String, Path), ("artifact_id" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = TagResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn tags(
    State(app): State<App>,
    Params((slug, artifact_id)): Params<(String, String)>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    ok(app.runtime.operations().tags(&slug, &artifact_id, &principal.checked()?).await?)
}

#[utoipa::path(put, path = "/v1/projects/{slug}/knowledge/{artifact_id}/tags/{tag}",
    operation_id = "add_knowledge_tag_v1_projects__slug__knowledge__artifact_id__tags__tag__put",
    summary = "Add Knowledge Tag",
    params(("slug" = String, Path), ("artifact_id" = String, Path), ("tag" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = TagResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn add_tag(
    State(app): State<App>,
    Params((slug, artifact_id, tag)): Params<(String, String, String)>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    let operations = app.runtime.operations();
    ok(operations.add_tag(&slug, &artifact_id, &tag, &principal.checked()?).await?)
}

#[utoipa::path(delete, path = "/v1/projects/{slug}/knowledge/{artifact_id}/tags/{tag}",
    operation_id = "remove_knowledge_tag_v1_projects__slug__knowledge__artifact_id__tags__tag__delete",
    summary = "Remove Knowledge Tag",
    params(("slug" = String, Path), ("artifact_id" = String, Path), ("tag" = String, Path), ActorHeader),
    responses((status = 200, description = "Successful Response", body = TagResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn remove_tag(
    State(app): State<App>,
    Params((slug, artifact_id, tag)): Params<(String, String, String)>,
    request: Request,
) -> ApiResult {
    let mut call = Call::new(parts(request));
    let principal = call.caller().await?;
    call.check()?;
    let operations = app.runtime.operations();
    ok(operations.remove_tag(&slug, &artifact_id, &tag, &principal.checked()?).await?)
}
