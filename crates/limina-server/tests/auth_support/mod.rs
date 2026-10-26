//! Shared helpers for the authentication tests: locally generated RSA keys that sign test JWTs, a
//! fake identity provider (WorkOS API, OpenID discovery and JWKS) on a loopback port, and a small
//! cookie-aware client for routers. Every value here is fake.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::rsa::{KeyPair, KeySize};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, State};
use axum::http::header::{COOKIE, HOST, SET_COOKIE};
use axum::http::{HeaderMap, Method, Request, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use jsonwebtoken::jwk::{Jwk, PublicKeyUse};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::{Value, json};
use tower::ServiceExt;

pub const CLIENT_ID: &str = "client_test123";
/// A fake cookie password (40 characters).
pub const PASSWORD: &str = "pppppppppppppppppppppppppppppppppppppppp";

// ─── Keys and tokens ──────────────────────────────────────────────────────────

/// An RSA key pair that signs test tokens, published under `kid`.
pub struct TestKey {
    pub kid: String,
    encoding: EncodingKey,
    jwk: Value,
}

impl TestKey {
    pub fn generate(kid: &str) -> Self {
        let pair = KeyPair::generate(KeySize::Rsa2048).unwrap();
        let pkcs8 = AsDer::<aws_lc_rs::encoding::Pkcs8V1Der>::as_der(&pair).unwrap();
        let encoding = EncodingKey::from_rsa_der(&pkcs1_from_pkcs8(pkcs8.as_ref()));
        let mut jwk = Jwk::from_encoding_key(&encoding, Algorithm::RS256).unwrap();
        jwk.common.key_id = Some(kid.to_owned());
        jwk.common.public_key_use = Some(PublicKeyUse::Signature);
        Self { kid: kid.to_owned(), encoding, jwk: serde_json::to_value(jwk).unwrap() }
    }

    /// The public key as a JWK (`kty`, `n`, `e`, `kid`, `use`, `alg`).
    pub fn jwk(&self) -> Value {
        self.jwk.clone()
    }

    pub fn sign(&self, claims: &Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.kid.clone());
        self.sign_with(&header, claims)
    }

    pub fn sign_with(&self, header: &Header, claims: &Value) -> String {
        jsonwebtoken::encode(header, claims, &self.encoding).unwrap()
    }
}

/// Two keys per test binary (generating RSA keys is the slow part).
pub static KEYS: LazyLock<[TestKey; 2]> =
    LazyLock::new(|| [TestKey::generate("key-1"), TestKey::generate("key-2")]);

/// The `RSAPrivateKey` inside a PKCS#8 `PrivateKeyInfo` (jsonwebtoken signs with PKCS#1 DER).
fn pkcs1_from_pkcs8(der: &[u8]) -> Vec<u8> {
    fn element(input: &[u8]) -> (u8, &[u8], &[u8]) {
        let tag = input[0];
        let (length, header) = if input[1] < 0x80 {
            (usize::from(input[1]), 2)
        } else {
            let count = usize::from(input[1] & 0x7f);
            let length =
                input[2..2 + count].iter().fold(0, |acc, byte| acc << 8 | usize::from(*byte));
            (length, 2 + count)
        };
        (tag, &input[header..header + length], &input[header + length..])
    }
    let (tag, info, _) = element(der);
    assert_eq!(tag, 0x30);
    let (_, _version, rest) = element(info);
    let (_, _algorithm, rest) = element(rest);
    let (tag, private_key, _) = element(rest);
    assert_eq!(tag, 0x04);
    private_key.to_vec()
}

pub fn now() -> i64 {
    i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()).unwrap()
}

/// A WorkOS-style access token (the Python tests' `access_token()` helper).
#[derive(Clone)]
pub struct AccessToken {
    pub sub: String,
    pub permissions: Vec<String>,
    pub org_id: Option<String>,
    pub expires_in: i64,
    pub issuer: String,
    pub key: usize,
}

impl AccessToken {
    pub fn new(issuer: &str) -> Self {
        Self {
            sub: "user_01".to_owned(),
            permissions: Vec::new(),
            org_id: Some("org_01".to_owned()),
            expires_in: 300,
            issuer: issuer.to_owned(),
            key: 0,
        }
    }

    pub fn permissions(mut self, permissions: &[&str]) -> Self {
        self.permissions = permissions.iter().map(|item| (*item).to_owned()).collect();
        self
    }

    pub fn org(mut self, org_id: Option<&str>) -> Self {
        self.org_id = org_id.map(str::to_owned);
        self
    }

    pub fn expires_in(mut self, seconds: i64) -> Self {
        self.expires_in = seconds;
        self
    }

    pub fn issuer(mut self, issuer: &str) -> Self {
        issuer.clone_into(&mut self.issuer);
        self
    }

    pub fn signed(&self) -> String {
        let now = now();
        let mut claims = json!({
            "iss": self.issuer,
            "sub": self.sub,
            "sid": "session_01",
            "iat": now,
            "exp": now + self.expires_in,
            "permissions": self.permissions,
        });
        if let Some(org_id) = &self.org_id {
            claims["org_id"] = json!(org_id);
        }
        KEYS[self.key].sign(&claims)
    }
}

// ─── Fake identity provider ───────────────────────────────────────────────────

/// One recorded call to the fake provider.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: Method,
    pub path: String,
    pub body: Value,
}

pub struct FakeState {
    /// Every POST, with its JSON body (token exchanges).
    pub requests: Vec<Recorded>,
    /// Every GET path (discovery and key sets).
    pub gets: Vec<String>,
    /// Permissions of the access tokens the code exchange and refresh return.
    pub permissions: Vec<String>,
    /// Make `/user_management/authenticate` fail.
    pub fail: bool,
    /// The client's OpenID configuration; 404 when `None`.
    pub discovery: Option<Value>,
    /// Issuer of the access tokens the provider returns.
    pub issuer: String,
    /// Indexes into [`KEYS`] published by every JWKS path.
    pub published: Vec<usize>,
    /// The key that signs returned access tokens.
    pub signing_key: usize,
}

/// A WorkOS API / OIDC provider stand-in on `http://127.0.0.1:<port>`.
pub struct FakeIdp {
    pub base_url: String,
    pub state: Arc<Mutex<FakeState>>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for FakeIdp {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl FakeIdp {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(FakeState {
            requests: Vec::new(),
            gets: Vec::new(),
            permissions: vec!["limina:access".to_owned()],
            fail: false,
            discovery: None,
            issuer: format!("{base_url}/user_management/{CLIENT_ID}"),
            published: vec![0],
            signing_key: 0,
        }));
        let app = Router::new().fallback(handle).with_state(state.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { base_url, state, server }
    }

    /// The API-host issuer for [`CLIENT_ID`] (the Python tests' `ISSUER`).
    pub fn issuer(&self) -> String {
        format!("{}/user_management/{CLIENT_ID}", self.base_url)
    }

    pub fn token(&self) -> AccessToken {
        AccessToken::new(&self.issuer())
    }

    pub fn with<T>(&self, change: impl FnOnce(&mut FakeState) -> T) -> T {
        change(&mut self.state.lock().unwrap())
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.with(|state| state.requests.clone())
    }

    pub fn gets(&self) -> Vec<String> {
        self.with(|state| state.gets.clone())
    }
}

async fn handle(
    State(state): State<Arc<Mutex<FakeState>>>,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> Response {
    let path = uri.path().to_owned();
    let mut state = state.lock().unwrap();
    if method == Method::GET {
        state.gets.push(path.clone());
        if path.ends_with("/openid-configuration") {
            return match &state.discovery {
                Some(discovery) => axum::Json(discovery.clone()).into_response(),
                None => (StatusCode::NOT_FOUND, axum::Json(json!({"message": "Not found"})))
                    .into_response(),
            };
        }
        if path.contains("jwks") {
            let keys: Vec<Value> = state.published.iter().map(|index| KEYS[*index].jwk()).collect();
            return axum::Json(json!({"keys": keys})).into_response();
        }
        return StatusCode::NOT_FOUND.into_response();
    }
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    state.requests.push(Recorded { method, path: path.clone(), body });
    if state.fail || path != "/user_management/authenticate" {
        return (StatusCode::BAD_REQUEST, axum::Json(json!({"error": "invalid_grant"})))
            .into_response();
    }
    let permissions: Vec<&str> = state.permissions.iter().map(String::as_str).collect();
    let mut token = AccessToken::new(&state.issuer).permissions(&permissions);
    token.key = state.signing_key;
    axum::Json(json!({
        "access_token": token.signed(),
        "refresh_token": format!("refresh_{}", state.requests.len()),
        "user": {
            "id": "user_01",
            "email": "ada@example.com",
            "first_name": "Ada",
            "last_name": "Lovelace",
        },
    }))
    .into_response()
}

// ─── A cookie-aware client for routers ────────────────────────────────────────

pub struct TestResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl TestResponse {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }

    pub fn location(&self) -> &str {
        self.headers.get("location").unwrap().to_str().unwrap()
    }

    pub fn set_cookies(&self) -> Vec<String> {
        self.headers
            .get_all(SET_COOKIE)
            .iter()
            .map(|value| value.to_str().unwrap().to_owned())
            .collect()
    }
}

/// Sends requests to a router as `http://testserver`, keeping cookies like a browser would.
pub struct TestClient {
    app: Router,
    /// name → (value, path)
    pub cookies: HashMap<String, (String, String)>,
    /// The peer address the server sees (`ConnectInfo`); none by default.
    pub peer: Option<SocketAddr>,
}

impl TestClient {
    pub fn new(app: Router) -> Self {
        Self { app, cookies: HashMap::new(), peer: None }
    }

    pub async fn get(&mut self, path: &str) -> TestResponse {
        self.send(Method::GET, path, &[]).await
    }

    pub async fn send(
        &mut self,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
    ) -> TestResponse {
        let mut request = Request::builder().method(method).uri(path);
        if !headers.iter().any(|(name, _)| name.eq_ignore_ascii_case("host")) {
            request = request.header(HOST, "testserver");
        }
        if let Some(peer) = self.peer {
            request = request.extension(ConnectInfo(peer));
        }
        let request_path = path.split('?').next().unwrap_or(path);
        let cookie = self
            .cookies
            .iter()
            .filter(|(_, (_, cookie_path))| path_matches(request_path, cookie_path))
            .map(|(name, (value, _))| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        if !cookie.is_empty() {
            request = request.header(COOKIE, cookie);
        }
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response =
            self.app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let response = TestResponse { status, headers, body };
        for set_cookie in response.set_cookies() {
            self.store(&set_cookie);
        }
        response
    }

    pub fn set_cookie(&mut self, name: &str, value: &str) {
        self.cookies.insert(name.to_owned(), (value.to_owned(), "/".to_owned()));
    }

    pub fn has_cookie(&self, name: &str) -> bool {
        self.cookies.contains_key(name)
    }

    fn store(&mut self, set_cookie: &str) {
        let mut parts = set_cookie.split(';').map(str::trim);
        let (name, value) = parts.next().unwrap().split_once('=').unwrap();
        let mut path = "/".to_owned();
        let mut expired = false;
        for attribute in parts {
            let (key, attribute_value) = attribute.split_once('=').unwrap_or((attribute, ""));
            match key.to_ascii_lowercase().as_str() {
                "path" => attribute_value.clone_into(&mut path),
                "max-age" => expired = attribute_value == "0",
                _ => {}
            }
        }
        if expired {
            self.cookies.remove(name);
        } else {
            self.cookies.insert(name.to_owned(), (value.to_owned(), path));
        }
    }
}

fn path_matches(request_path: &str, cookie_path: &str) -> bool {
    request_path == cookie_path
        || (request_path.starts_with(cookie_path)
            && (cookie_path.ends_with('/') || request_path[cookie_path.len()..].starts_with('/')))
}

/// The query parameters of a URL (last value wins).
pub fn query(url: &str) -> HashMap<String, String> {
    let query = url.split_once('?').map_or("", |(_, query)| query);
    url::form_urlencoded::parse(query.as_bytes()).into_owned().collect()
}
