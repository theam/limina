//! The cross-site request rule for cookie credentials.
//!
//! Browsers attach cookies to cross-site requests, so a session cookie alone cannot prove that a
//! write came from the console. A state-changing request (any method except GET, HEAD and OPTIONS)
//! that authenticates with the session cookie must carry an `Origin` header equal to the console's
//! public origin (`LIMINA_PUBLIC_URL`, or the request's own origin when unset). Bearer credentials
//! are not ambient and are not subject to this rule. The cookie is also `SameSite=Lax`.

use axum::http::header::{HOST, HeaderMap, ORIGIN};
use axum::http::{Method, Uri};
use limina_core::{LiminaError, Result};

/// Methods that must not change state, so cookies may authenticate them from anywhere.
pub const SAFE_METHODS: [Method; 3] = [Method::GET, Method::HEAD, Method::OPTIONS];

pub fn is_safe_method(method: &Method) -> bool {
    SAFE_METHODS.contains(method)
}

/// Rejects a state-changing, cookie-authenticated request whose `Origin` is not the console's.
pub fn require_console_origin(
    method: &Method,
    origin: Option<&str>,
    public_origin: &str,
) -> Result<()> {
    if is_safe_method(method) || origin == Some(public_origin) {
        return Ok(());
    }
    Err(LiminaError::authorization_with("Cross-origin requests cannot use the browser session.")
        .with("reason", "origin"))
}

/// `scheme://netloc` of a URL, as Python's `urlsplit` splits it (the scheme is lowercased; the
/// host and port are kept verbatim).
pub fn origin_of(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return "://".to_owned();
    };
    let netloc = rest.split(['/', '?', '#']).next().unwrap_or_default();
    format!("{}://{netloc}", scheme.to_ascii_lowercase())
}

/// The origin the request was sent to: `scheme` (see [`TrustedProxies::resolve`]) and the URI
/// authority (HTTP/2) or the `Host` header.
///
/// [`TrustedProxies::resolve`]: super::proxy::TrustedProxies::resolve
pub fn request_origin(scheme: &str, uri: &Uri, headers: &HeaderMap) -> String {
    let host = uri
        .authority()
        .map(|authority| authority.as_str().to_owned())
        .or_else(|| {
            headers.get(HOST).map(|host| String::from_utf8_lossy(host.as_bytes()).into_owned())
        })
        .unwrap_or_default();
    format!("{scheme}://{host}")
}

/// The request's `Origin` header, if any.
pub fn origin_header(headers: &HeaderMap) -> Option<String> {
    headers.get(ORIGIN).map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_state_changing_requests_need_the_console_origin() {
        let console = "https://limina.example.com";
        assert!(require_console_origin(&Method::GET, None, console).is_ok());
        assert!(
            require_console_origin(&Method::HEAD, Some("https://evil.example"), console).is_ok()
        );
        assert!(require_console_origin(&Method::POST, Some(console), console).is_ok());
        for origin in
            [None, Some("https://evil.example"), Some("null"), Some("http://limina.example.com")]
        {
            let error = require_console_origin(&Method::POST, origin, console).unwrap_err();
            assert_eq!((error.http_status(), error.code()), (403, "permission_denied"));
            assert_eq!(error.details["reason"], "origin");
        }
        assert!(require_console_origin(&Method::DELETE, None, console).is_err());
    }

    #[test]
    fn origins_follow_urlsplit() {
        assert_eq!(
            origin_of("HTTPS://Limina.example.com:8443/console?x#y"),
            "https://Limina.example.com:8443"
        );
        assert_eq!(origin_of("http://localhost:7433"), "http://localhost:7433");
        let mut headers = HeaderMap::new();
        headers.insert(HOST, "testserver".parse().unwrap());
        let relative = Uri::from_static("/auth/login");
        assert_eq!(request_origin("http", &relative, &headers), "http://testserver");
        assert_eq!(request_origin("https", &relative, &headers), "https://testserver");
        let absolute = Uri::from_static("https://limina.example.com/v1/me");
        assert_eq!(request_origin("https", &absolute, &headers), "https://limina.example.com");
    }
}
