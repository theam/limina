//! The credentials a request carries, read once from its method and headers.

use std::net::{IpAddr, SocketAddr};

use axum::extract::ConnectInfo;
use axum::http::header::{AUTHORIZATION, HeaderMap};
use axum::http::request::Parts;
use axum::http::{Method, Uri};

use super::cookies::{SESSION_COOKIE, request_cookie};
use super::origin::{origin_header, request_origin};
use super::proxy::TrustedProxies;

/// The caller-supplied display name for static-token principals.
pub const ACTOR_HEADER: &str = "x-limina-actor";
/// The longest accepted `X-Limina-Actor` (characters), as the REST API validates it.
pub const ACTOR_LIMIT: usize = 200;

/// Everything authentication looks at in a request.
#[derive(Clone)]
pub struct RequestCredentials {
    pub method: Method,
    /// From `Authorization: Bearer <token>` (scheme case-insensitive).
    pub bearer_token: Option<String>,
    /// The `limina_session` cookie.
    pub session_cookie: Option<String>,
    /// `X-Limina-Actor`, verbatim.
    pub actor_hint: Option<String>,
    /// The `Origin` header.
    pub origin: Option<String>,
    /// `scheme://host` the request was sent to (see [`request_origin`]).
    pub request_origin: String,
    /// The client address: the peer recorded by the server (`into_make_service_with_connect_info`),
    /// or the forwarded client when the peer is a trusted proxy.
    pub client: Option<IpAddr>,
}

impl std::fmt::Debug for RequestCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestCredentials")
            .field("method", &self.method)
            .field("bearer_token", &self.bearer_token.as_ref().map(|_| "<redacted>"))
            .field("session_cookie", &self.session_cookie.as_ref().map(|_| "<redacted>"))
            .field("actor_hint", &self.actor_hint)
            .field("origin", &self.origin)
            .field("request_origin", &self.request_origin)
            .field("client", &self.client)
            .finish()
    }
}

impl RequestCredentials {
    /// Reads the request, taking the peer address from axum's `ConnectInfo<SocketAddr>`.
    pub fn from_parts(parts: &Parts, proxies: &TrustedProxies) -> Self {
        Self::from_headers(&parts.method, &parts.uri, &parts.headers, peer_address(parts), proxies)
    }

    pub fn from_headers(
        method: &Method,
        uri: &Uri,
        headers: &HeaderMap,
        peer: Option<IpAddr>,
        proxies: &TrustedProxies,
    ) -> Self {
        let (scheme, client) = proxies.resolve(peer, uri, headers);
        Self {
            method: method.clone(),
            bearer_token: bearer_token(headers),
            session_cookie: request_cookie(headers, SESSION_COOKIE),
            actor_hint: actor_hint(headers),
            origin: origin_header(headers),
            request_origin: request_origin(&scheme, uri, headers),
            client,
        }
    }

    /// The per-client failure-limit key: the peer IP address, or `unknown`.
    pub fn client_key(&self) -> String {
        self.client.map_or_else(|| "unknown".to_owned(), |address| address.to_string())
    }
}

/// The peer address axum recorded (`into_make_service_with_connect_info::<SocketAddr>()`).
pub fn peer_address(parts: &Parts) -> Option<IpAddr> {
    parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|info| info.0.ip())
}

/// The credential of an `Authorization: Bearer <credential>` header (the first header; the scheme
/// is case-insensitive; an empty credential counts as none).
pub fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = String::from_utf8_lossy(headers.get(AUTHORIZATION)?.as_bytes()).into_owned();
    let (scheme, credential) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !credential.is_empty()).then(|| credential.to_owned())
}

/// `X-Limina-Actor`, verbatim (decoded as UTF-8, invalid bytes replaced).
pub fn actor_hint(headers: &HeaderMap) -> Option<String> {
    headers.get(ACTOR_HEADER).map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn bearer_credentials_follow_the_http_bearer_scheme() {
        let parse = |value: &'static str| {
            let mut headers = HeaderMap::new();
            headers.insert(AUTHORIZATION, HeaderValue::from_static(value));
            bearer_token(&headers)
        };
        assert_eq!(parse("Bearer abc").as_deref(), Some("abc"));
        assert_eq!(parse("bearer abc").as_deref(), Some("abc"));
        assert_eq!(parse("Bearer "), None);
        assert_eq!(parse("Basic abc"), None);
        assert_eq!(parse("Bearer"), None);
        assert_eq!(bearer_token(&HeaderMap::new()), None);
    }
}
