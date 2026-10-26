//! The web console, served next to the API from a static build (`index.html` plus hashed
//! `/assets`), as `web_ui.py` did.
//!
//! - `/assets/*` are content-hashed and cached forever (`public, max-age=31536000, immutable`).
//! - Any other `GET`/`HEAD` of a path no route serves gets the file of that name from the build,
//!   or `index.html` (`no-cache`) so client-side routes like `/projects/x/conversation` load the
//!   console. A missing file with an extension (`/favicon.ico`) is a 404, not the app shell.
//! - Paths that belong to the API or its tooling (`/v1`, `/internal`, `/auth`, `/mcp`, the
//!   probes, `/docs`, `/openapi.json`, …) never fall back to the console: they keep FastAPI's
//!   JSON 404 (`{"detail": "Not Found"}`), as does every unknown path without a build.
//! - Files resolve strictly inside the build directory: `..`, encoded traversal and symlinks
//!   that point elsewhere are refused.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH,
    LAST_MODIFIED,
};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};

use super::error::not_found;

/// First path segments that belong to the API or its tooling.
pub const RESERVED_SEGMENTS: [&str; 11] = [
    "v1",
    "auth",
    "internal",
    "mcp",
    "healthz",
    "readyz",
    "livez",
    "docs",
    "redoc",
    "openapi.json",
    "assets",
];
pub const REVALIDATE: &str = "no-cache";
pub const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// The console build to serve: `configured` when it holds an `index.html` (canonicalized), or
/// `None` for an API-only runtime (with a warning when a directory was configured).
pub fn resolve_web_dir(configured: Option<&Path>) -> Option<PathBuf> {
    let configured = configured?;
    if configured.join("index.html").is_file()
        && let Ok(resolved) = configured.canonicalize()
    {
        return Some(resolved);
    }
    tracing::warn!("Limina web UI disabled: {} has no index.html.", configured.display());
    None
}

/// The console's files, or none for an API-only runtime.
#[derive(Debug, Clone, Default)]
pub struct Console {
    root: Option<PathBuf>,
}

impl Console {
    /// `root` must already be resolved ([`resolve_web_dir`]).
    pub fn new(root: Option<PathBuf>) -> Self {
        Self { root }
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }
}

/// `GET /assets/{*path}`: a build asset, cached forever.
pub async fn asset(State(console): State<Console>, request: Request) -> Response {
    let Some(root) = console.root() else { return not_found() };
    let Ok(assets) = tokio::fs::canonicalize(root.join("assets")).await else {
        return not_found();
    };
    let path = decoded_path(request.uri().path());
    let relative = path.strip_prefix("/assets").unwrap_or(&path);
    let Some(target) = static_file(&assets, relative).await else { return not_found() };
    file_response(&target, IMMUTABLE, request.method(), request.headers()).await
}

/// Every path no route serves: a console file or the app shell, or the JSON 404.
pub async fn fallback(State(console): State<Console>, request: Request) -> Response {
    let method = request.method();
    let path = decoded_path(request.uri().path());
    let Some(root) = console.root() else { return not_found() };
    if (method != Method::GET && method != Method::HEAD) || is_reserved(&path) {
        return not_found();
    }
    let target = match static_file(root, &path).await {
        Some(target) => target,
        None if path.rsplit('/').next().unwrap_or_default().contains('.') => return not_found(),
        None => root.join("index.html"),
    };
    file_response(&target, REVALIDATE, method, request.headers()).await
}

/// Whether the path's first segment belongs to the API.
pub fn is_reserved(path: &str) -> bool {
    let first = path.trim_start_matches('/').split('/').next().unwrap_or_default();
    RESERVED_SEGMENTS.contains(&first)
}

/// The request path percent-decoded (as Starlette routed it), invalid UTF-8 replaced.
pub fn decoded_path(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = |offset: usize| {
            bytes.get(index + offset).and_then(|byte| char::from(*byte).to_digit(16))
        };
        match (bytes[index], hex(1), hex(2)) {
            (b'%', Some(high), Some(low)) => {
                decoded.push(u8::try_from(high * 16 + low).unwrap_or(b'?'));
                index += 3;
            }
            (byte, _, _) => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// The regular file `path` names strictly inside `root` (symlinks resolved), if any.
pub async fn static_file(root: &Path, path: &str) -> Option<PathBuf> {
    let relative = path.trim_start_matches('/');
    if relative.is_empty() || relative.contains('\0') {
        return None;
    }
    let candidate = tokio::fs::canonicalize(root.join(relative)).await.ok()?;
    if !candidate.starts_with(root) {
        return None;
    }
    let metadata = tokio::fs::metadata(&candidate).await.ok()?;
    metadata.is_file().then_some(candidate)
}

async fn file_response(
    path: &Path,
    cache: &'static str,
    method: &Method,
    request: &HeaderMap,
) -> Response {
    let Ok(metadata) = tokio::fs::metadata(path).await else { return not_found() };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .unwrap_or_default();
    let etag = format!("\"{:x}-{:x}\"", modified.as_nanos(), metadata.len());
    let last_modified = httpdate(modified.as_secs());
    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(media_type(path)));
    if let Ok(value) = HeaderValue::from_str(&etag) {
        headers.insert(ETAG, value);
    }
    if let Ok(value) = HeaderValue::from_str(&last_modified) {
        headers.insert(LAST_MODIFIED, value);
    }
    let fresh = match request.get(IF_NONE_MATCH) {
        Some(tags) => String::from_utf8_lossy(tags.as_bytes())
            .split(',')
            .any(|tag| tag.trim().trim_start_matches("W/") == etag),
        None => request
            .get(IF_MODIFIED_SINCE)
            .is_some_and(|since| since.as_bytes() == last_modified.as_bytes()),
    };
    if fresh {
        headers.remove(CONTENT_TYPE);
        return (StatusCode::NOT_MODIFIED, headers).into_response();
    }
    headers.insert(CONTENT_LENGTH, HeaderValue::from(metadata.len()));
    if method == Method::HEAD {
        return (StatusCode::OK, headers, Body::empty()).into_response();
    }
    match tokio::fs::read(path).await {
        Ok(contents) => (StatusCode::OK, headers, contents).into_response(),
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "cannot read a console file");
            not_found()
        }
    }
}

/// The media type of a console file, by extension (text types in UTF-8).
pub fn media_type(path: &Path) -> &'static str {
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or_default();
    match extension.to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "txt" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/vnd.microsoft.icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// An HTTP date (`Tue, 15 Nov 1994 08:12:31 GMT`) for seconds since the Unix epoch.
fn httpdate(seconds: u64) -> String {
    let timestamp = i64::try_from(seconds).unwrap_or_default();
    chrono::DateTime::from_timestamp(timestamp, 0)
        .unwrap_or_default()
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_decode_and_reserved_segments_are_recognised() {
        assert_eq!(decoded_path("/%2e%2e/secret.txt"), "/../secret.txt");
        assert_eq!(decoded_path("/..%2fsecret.txt"), "/../secret.txt");
        assert_eq!(decoded_path("/100%"), "/100%");
        assert!(is_reserved("/v1/unknown"));
        assert!(is_reserved("/docs/unknown"));
        assert!(is_reserved("/openapi.json"));
        assert!(!is_reserved("/projects/retrieval"));
        assert!(!is_reserved("/"));
        assert_eq!(httpdate(784_887_151), "Tue, 15 Nov 1994 08:12:31 GMT");
    }

    #[tokio::test]
    async fn static_files_stay_inside_the_build() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("dist");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("favicon.svg"), "<svg/>").unwrap();
        std::fs::write(dir.path().join("secret.txt"), "secret").unwrap();
        let root = root.canonicalize().unwrap();
        assert_eq!(static_file(&root, "/favicon.svg").await, Some(root.join("favicon.svg")));
        assert_eq!(static_file(&root, "/../secret.txt").await, None);
        assert_eq!(static_file(&root, "/bad\0name").await, None);
        assert_eq!(static_file(&root, "/").await, None);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path().join("secret.txt"), root.join("escape.txt"))
                .unwrap();
            assert_eq!(static_file(&root, "/escape.txt").await, None);
        }
    }
}
