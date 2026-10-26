//! Shared helpers for the command-line tests: the built `limina` binary with a clean
//! environment, and a fake Limina instance that records requests and answers from a closure.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use tokio::sync::oneshot;

/// The `limina` binary without any `LIMINA_*` configuration from the developer's shell.
pub fn limina() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_limina"));
    for (key, _) in std::env::vars_os() {
        let key_text = key.to_string_lossy();
        if key_text.starts_with("LIMINA_")
            || key_text.starts_with("WORKOS_")
            || key_text == "NO_COLOR"
            || key_text == "RUST_LOG"
        {
            command.env_remove(&key);
        }
    }
    command.env("USER", "tester");
    command
}

/// A finished command.
#[derive(Debug)]
pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    /// Standard output parsed as one JSON document.
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|error| panic!("stdout is not JSON ({error}): {self:?}"))
    }

    pub fn output(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

pub fn run(command: &mut Command) -> Run {
    run_with_stdin(command, "")
}

pub fn run_with_stdin(command: &mut Command, input: &str) -> Run {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the limina binary starts");
    {
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(input.as_bytes()).unwrap();
    }
    let output = child.wait_with_output().unwrap();
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

/// One request the fake instance received.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    /// Lowercase header names.
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_lowercase()).map(String::as_str)
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }

    pub fn query_pairs(&self) -> BTreeMap<String, String> {
        self.query
            .as_deref()
            .unwrap_or_default()
            .split('&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| {
                let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                (key.to_owned(), value.to_owned())
            })
            .collect()
    }
}

/// What the fake instance answers.
#[derive(Debug, Clone)]
pub enum Reply {
    Json(u16, Value),
    Text(u16, String),
}

pub fn ok(value: Value) -> Reply {
    Reply::Json(200, value)
}

type Responder = dyn Fn(&Recorded) -> Reply + Send + Sync;

#[derive(Clone)]
struct ServerState {
    log: Arc<Mutex<Vec<Recorded>>>,
    responder: Arc<Responder>,
}

/// A local HTTP server standing in for a Limina instance.
pub struct FakeServer {
    pub url: String,
    log: Arc<Mutex<Vec<Recorded>>>,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl FakeServer {
    pub fn start(responder: impl Fn(&Recorded) -> Reply + Send + Sync + 'static) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let log = Arc::new(Mutex::new(Vec::new()));
        let state = ServerState { log: Arc::clone(&log), responder: Arc::new(responder) };
        let (shutdown, stopped) = oneshot::channel::<()>();
        let thread = std::thread::spawn(move || {
            let runtime =
                tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                let app = Router::new().fallback(handle).with_state(state);
                axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = stopped.await;
                    })
                    .await
                    .unwrap();
            });
        });
        Self { url, log, shutdown: Some(shutdown), thread: Some(thread) }
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.log.lock().unwrap().clone()
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

async fn handle(
    State(state): State<ServerState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let recorded = Recorded {
        method: method.to_string(),
        path: uri.path().to_owned(),
        query: uri.query().map(str::to_owned),
        headers: headers
            .iter()
            .map(|(name, value)| {
                (name.as_str().to_owned(), String::from_utf8_lossy(value.as_bytes()).into_owned())
            })
            .collect(),
        body: body.to_vec(),
    };
    state.log.lock().unwrap().push(recorded.clone());
    match (state.responder)(&recorded) {
        Reply::Json(status, value) => {
            (StatusCode::from_u16(status).unwrap(), axum::Json(value)).into_response()
        }
        Reply::Text(status, text) => (StatusCode::from_u16(status).unwrap(), text).into_response(),
    }
}

/// A Python `json.dumps` error body.
pub fn error_body(code: &str, message: &str, details: Value) -> Value {
    serde_json::json!({"error": {"code": code, "message": message, "details": details}})
}
