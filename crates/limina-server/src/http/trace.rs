//! One `tracing` span and one completion record per request: method, route template, status and
//! latency. Never the query string, headers, cookies, tokens or bodies (the route template
//! instead of the path keeps identifiers out of the logs too).

use std::time::Instant;

use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;
use tracing::Instrument;

/// The middleware (`axum::middleware::from_fn(trace::request)`).
pub async fn request(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "<unmatched>".to_owned(), |path| path.as_str().to_owned());
    let span = tracing::info_span!("http", %method, route = %route);
    let started = Instant::now();
    let response = next.run(request).instrument(span.clone()).await;
    let status = response.status().as_u16();
    let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    span.in_scope(|| {
        if status >= 500 {
            tracing::warn!(status, latency_ms, "request failed");
        } else {
            tracing::info!(status, latency_ms, "request");
        }
    });
    response
}
