//! Reading request cookies and writing `Set-Cookie` headers in the format the Python runtime
//! (Starlette) produced: `name=value; HttpOnly; Max-Age=N; Path=P; SameSite=lax[; Secure]`.

use axum::http::header::{COOKIE, HeaderMap, HeaderValue};
use limina_core::{Result, internal};

/// The session cookie: the sealed WorkOS session.
pub const SESSION_COOKIE: &str = "limina_session";
/// The sign-in flow cookie: sealed `state`, PKCE verifier and return path, scoped to `/auth`.
pub const FLOW_COOKIE: &str = "limina_auth_flow";
pub const FLOW_TTL_SECONDS: u64 = 600;
pub const SESSION_TTL_SECONDS: u64 = 30 * 24 * 3600;

/// The named cookie from every `Cookie` header; a later duplicate wins.
pub fn request_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let mut found = None;
    for header in headers.get_all(COOKIE) {
        let text = String::from_utf8_lossy(header.as_bytes());
        for chunk in text.split(';') {
            let (key, value) = chunk.split_once('=').unwrap_or(("", chunk));
            if key.trim() == name {
                found = Some(unquote(value.trim()).to_owned());
            }
        }
    }
    found
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .filter(|inner| !inner.contains(['"', '\\']))
        .unwrap_or(value)
}

/// An HttpOnly, SameSite=Lax cookie.
pub fn set_cookie(
    name: &str,
    value: &str,
    max_age_seconds: u64,
    path: &str,
    secure: bool,
) -> Result<HeaderValue> {
    let secure = if secure { "; Secure" } else { "" };
    header(format!(
        "{name}={value}; HttpOnly; Max-Age={max_age_seconds}; Path={path}; SameSite=lax{secure}"
    ))
}

/// Expires a cookie set on `path`.
pub fn delete_cookie(name: &str, path: &str) -> Result<HeaderValue> {
    header(format!(
        "{name}=\"\"; expires=Thu, 01 Jan 1970 00:00:00 GMT; Max-Age=0; Path={path}; SameSite=lax"
    ))
}

fn header(value: String) -> Result<HeaderValue> {
    HeaderValue::try_from(value).map_err(internal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies_are_read_like_starlette() {
        let mut headers = HeaderMap::new();
        headers.append(COOKIE, HeaderValue::from_static("a=1; limina_session=first"));
        headers.append(COOKIE, HeaderValue::from_static(" limina_session = \"second\" ;b"));
        assert_eq!(request_cookie(&headers, SESSION_COOKIE).as_deref(), Some("second"));
        assert_eq!(request_cookie(&headers, "a").as_deref(), Some("1"));
        assert_eq!(request_cookie(&headers, "missing"), None);
    }

    #[test]
    fn cookies_are_written_like_starlette() {
        assert_eq!(
            set_cookie(FLOW_COOKIE, "v", 600, "/auth", false).unwrap(),
            "limina_auth_flow=v; HttpOnly; Max-Age=600; Path=/auth; SameSite=lax"
        );
        assert_eq!(
            set_cookie(SESSION_COOKIE, "v", SESSION_TTL_SECONDS, "/", true).unwrap(),
            "limina_session=v; HttpOnly; Max-Age=2592000; Path=/; SameSite=lax; Secure"
        );
        assert_eq!(
            delete_cookie(SESSION_COOKIE, "/").unwrap(),
            "limina_session=\"\"; expires=Thu, 01 Jan 1970 00:00:00 GMT; Max-Age=0; Path=/; \
             SameSite=lax"
        );
    }
}
