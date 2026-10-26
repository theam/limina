//! Personal connections and explicit project permission. Credentials are never response fields.
use axum::http::header::SET_COOKIE;
use axum::{
    Json,
    extract::{Request, State},
    response::{IntoResponse, Redirect},
};
use limina_core::{LiminaError, new_uuid};
use serde::Deserialize;

use super::{Params, ok};
use crate::http::{App, error::ApiResult, schemas::PublicErrors};
use crate::{
    app::connections::{ConnectionChanges, NewConnection, oauth::flow_cookie},
    auth::{
        Caller,
        cookies::{delete_cookie, request_cookie, set_cookie},
    },
};

#[derive(Default, Deserialize, utoipa::ToSchema)]
pub struct ConnectRequest {
    pub return_to: Option<String>,
}

#[utoipa::path(get,path="/v1/connections",responses((status=200,body=Vec<Value>),PublicErrors),security(("HTTPBearer"=[])))]
pub async fn list(State(app): State<App>, Caller(person): Caller) -> ApiResult {
    ok(app.runtime.supervisor().connections().list(&person).await?)
}

#[utoipa::path(post,path="/v1/connections",request_body=NewConnection,responses((status=200,body=Value),PublicErrors),security(("HTTPBearer"=[])))]
pub async fn create(
    State(app): State<App>,
    Caller(person): Caller,
    headers: axum::http::HeaderMap,
    Json(body): Json<NewConnection>,
) -> ApiResult {
    let command = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= 55)
        .ok_or_else(|| {
            LiminaError::invariant("An Idempotency-Key of 1 to 55 characters is required.")
        })?;
    ok(app.runtime.supervisor().connections().create(body, &person, command).await?)
}

#[utoipa::path(get,path="/v1/connections/{id}",params(("id"=String,Path)),responses((status=200,body=Value),PublicErrors),security(("HTTPBearer"=[])))]
pub async fn get(
    State(app): State<App>,
    Params(id): Params<String>,
    Caller(person): Caller,
) -> ApiResult {
    ok(app.runtime.supervisor().connections().get(&id, &person).await?)
}

#[utoipa::path(patch,path="/v1/connections/{id}",params(("id"=String,Path)),request_body=ConnectionChanges,responses((status=200,body=Value),PublicErrors),security(("HTTPBearer"=[])))]
pub async fn update(
    State(app): State<App>,
    Params(id): Params<String>,
    Caller(person): Caller,
    Json(body): Json<ConnectionChanges>,
) -> ApiResult {
    ok(app.runtime.supervisor().connections().update(&id, body, &person).await?)
}

#[utoipa::path(delete,path="/v1/connections/{id}",params(("id"=String,Path)),responses((status=200,body=Value),PublicErrors),security(("HTTPBearer"=[])))]
pub async fn remove(
    State(app): State<App>,
    Params(id): Params<String>,
    Caller(person): Caller,
) -> ApiResult {
    ok(app.runtime.supervisor().connections().remove(&id, &person, true).await?)
}

#[utoipa::path(post,path="/v1/connections/{id}/disconnect",params(("id"=String,Path)),responses((status=200,body=Value),PublicErrors),security(("HTTPBearer"=[])))]
pub async fn disconnect(
    State(app): State<App>,
    Params(id): Params<String>,
    Caller(person): Caller,
) -> ApiResult {
    ok(app.runtime.supervisor().connections().remove(&id, &person, false).await?)
}

#[utoipa::path(post,path="/v1/connections/{id}/connect",params(("id"=String,Path)),request_body=ConnectRequest,responses((status=200,body=Value),PublicErrors),security(("HTTPBearer"=[])))]
pub async fn connect(
    State(app): State<App>,
    Params(id): Params<String>,
    Caller(person): Caller,
    request: Request,
) -> ApiResult {
    let (parts, body) = request.into_parts();
    let bytes = crate::http::call::read_body(body, app.settings.json_body_limit_bytes).await?;
    let body: ConnectRequest = if bytes.is_empty() {
        ConnectRequest::default()
    } else {
        serde_json::from_slice(&bytes)
            .map_err(|_| LiminaError::invariant("Invalid connect request."))?
    };
    let credentials = app.auth.credentials(&parts);
    let origin = app
        .auth
        .sessions()
        .map(|sessions| sessions.public_origin(&credentials.request_origin))
        .unwrap_or(credentials.request_origin);
    let redirect = format!("{}/auth/mcp/callback", origin.trim_end_matches('/'));
    let browser = new_uuid();
    let outcome = app
        .runtime
        .supervisor()
        .connections()
        .connect(
            &id,
            &person,
            &redirect,
            body.return_to.as_deref().unwrap_or("/connections"),
            &browser,
        )
        .await?;
    let mut response = Json(outcome.body).into_response();
    if let Some(state) = outcome.state {
        response.headers_mut().append(
            SET_COOKIE,
            set_cookie(
                &flow_cookie(&state),
                &browser,
                600,
                "/auth/mcp",
                origin.starts_with("https://"),
            )?,
        );
    }
    Ok(response)
}

#[utoipa::path(get,path="/v1/projects/{slug}/connections",params(("slug"=String,Path)),responses((status=200,body=Value),PublicErrors),security(("HTTPBearer"=[])))]
pub async fn project(
    State(app): State<App>,
    Params(slug): Params<String>,
    Caller(person): Caller,
) -> ApiResult {
    ok(app.runtime.supervisor().connections().for_project(&slug, &person).await?)
}

#[utoipa::path(put,path="/v1/projects/{slug}/connections/{id}",params(("slug"=String,Path),("id"=String,Path)),responses((status=200,body=Value),PublicErrors),security(("HTTPBearer"=[])))]
pub async fn enable(
    State(app): State<App>,
    Params((slug, id)): Params<(String, String)>,
    Caller(person): Caller,
) -> ApiResult {
    ok(app.runtime.supervisor().connections().enable(&slug, &id, &person).await?)
}

#[utoipa::path(delete,path="/v1/projects/{slug}/connections/{id}",params(("slug"=String,Path),("id"=String,Path)),responses((status=200,body=Value),PublicErrors),security(("HTTPBearer"=[])))]
pub async fn disable(
    State(app): State<App>,
    Params((slug, id)): Params<(String, String)>,
    Caller(person): Caller,
) -> ApiResult {
    ok(app.runtime.supervisor().connections().disable(&slug, &id, &person).await?)
}

pub async fn callback(State(app): State<App>, request: Request) -> ApiResult {
    let query = url::form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
        .into_owned()
        .collect::<std::collections::BTreeMap<_, _>>();
    let state = query
        .get("state")
        .ok_or_else(|| LiminaError::invariant("The authorization state is missing."))?;
    let cookie = flow_cookie(state);
    let browser = request_cookie(request.headers(), &cookie);
    let result = app
        .runtime
        .supervisor()
        .connections()
        .callback(
            state,
            browser.as_deref(),
            query.get("code").map(String::as_str),
            query.get("iss").map(String::as_str),
            query.contains_key("error"),
        )
        .await?;
    let mut target = url::Url::parse("http://local.invalid")
        .and_then(|base| base.join(&result.return_to))
        .map_err(limina_core::internal)?;
    if let Some(error) = result.error {
        target
            .query_pairs_mut()
            .append_pair("connection_error", error)
            .append_pair("connection", &result.id);
    } else {
        target.query_pairs_mut().append_pair("connected", &result.id);
    }
    let location =
        format!("{}{}", target.path(), target.query().map(|q| format!("?{q}")).unwrap_or_default());
    let mut response = Redirect::to(&location).into_response();
    response.headers_mut().append(SET_COOKIE, delete_cookie(&cookie, "/auth/mcp")?);
    Ok(response)
}
