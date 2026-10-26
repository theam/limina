//! The OpenAPI document (`/openapi.json`) and its viewers (`/docs`, `/redoc`).
//!
//! The document is generated from the route annotations (`#[utoipa::path]`) and the request and
//! response types of [`super::requests`] and [`super::schemas`]; [`super::routes::public`]
//! registers each documented handler with axum from the same annotation, so a route cannot be
//! served without being documented or documented without being served. Internal routes, the live
//! WebSocket, sign-in and the console are not part of it, as in the Python API.

use axum::http::header::CONTENT_TYPE;
use axum::response::{Html, IntoResponse, Response};
use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};
use utoipa::{Modify, OpenApi};

use super::schemas::{ErrorDetail, ErrorResponse};

pub const TITLE: &str = "Limina Project Runtime";
pub const DESCRIPTION: &str =
    "Mission, review, resource, steering, and lifecycle control for Limina-owned runtimes.";

#[derive(OpenApi)]
#[openapi(
    info(title = "Limina Project Runtime", description = "Mission, review, resource, steering, and lifecycle control for Limina-owned runtimes."),
    components(schemas(ErrorResponse, ErrorDetail)),
    modifiers(&BearerScheme)
)]
pub struct ApiDoc;

/// `HTTPBearer`, the scheme FastAPI's `HTTPBearer` dependency published.
struct BearerScheme;

impl Modify for BearerScheme {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "HTTPBearer",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
    }
}

/// The base document: title, description, version and the shared components.
pub fn base() -> utoipa::openapi::OpenApi {
    let mut document = ApiDoc::openapi();
    document.info.version = env!("CARGO_PKG_VERSION").to_owned();
    document.info.license = None;
    document.info.contact = None;
    document
}

/// The document as served: rendered once, answered as is.
#[derive(Clone)]
pub struct Document(std::sync::Arc<str>);

impl Document {
    pub fn new(document: &utoipa::openapi::OpenApi) -> Self {
        let rendered = serde_json::to_string(document).unwrap_or_else(|_| "{}".to_owned());
        Self(rendered.into())
    }

    pub fn json(&self) -> &str {
        &self.0
    }
}

impl IntoResponse for Document {
    fn into_response(self) -> Response {
        ([(CONTENT_TYPE, "application/json")], self.0.to_string()).into_response()
    }
}

/// Swagger UI for `/openapi.json` (the page FastAPI served at `/docs`).
pub async fn swagger_ui() -> Html<String> {
    Html(format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<link type="text/css" rel="stylesheet" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui.css">
<title>{TITLE} - Swagger UI</title>
</head>
<body>
<div id="swagger-ui"></div>
<script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
<script>
const ui = SwaggerUIBundle({{
  url: '/openapi.json',
  dom_id: '#swagger-ui',
  layout: 'BaseLayout',
  deepLinking: true,
  showExtensions: true,
  showCommonExtensions: true,
  presets: [SwaggerUIBundle.presets.apis, SwaggerUIBundle.SwaggerUIStandalonePreset],
}})
</script>
</body>
</html>
"#
    ))
}

/// ReDoc for `/openapi.json` (FastAPI's `/redoc`).
pub async fn redoc() -> Html<String> {
    Html(format!(
        r#"<!DOCTYPE html>
<html>
<head>
<title>{TITLE} - ReDoc</title>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>body {{ margin: 0; padding: 0; }}</style>
</head>
<body>
<noscript>ReDoc requires Javascript to function. Please enable it to browse the documentation.</noscript>
<redoc spec-url="/openapi.json"></redoc>
<script src="https://cdn.jsdelivr.net/npm/redoc@2/bundles/redoc.standalone.js"></script>
</body>
</html>
"#
    ))
}
