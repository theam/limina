//! The live project channel: `GET /v1/projects/{slug}/live` upgraded to a WebSocket.
//!
//! # Handshake
//!
//! The client offers the `limina.v1` subprotocol and authenticates with either one
//! `limina.ticket.<ticket>` subprotocol (a single-use ticket from
//! `POST /v1/projects/{slug}/live-ticket`, how browsers connect) or an `Authorization: Bearer`
//! header (CLI, automation; `X-Limina-Actor` names static-token callers). Refusals close the
//! socket right after the handshake:
//!
//! | Code | When |
//! |---|---|
//! | 4400 | `limina.v1` was not offered, or more than one ticket was |
//! | 4401 | no valid credential, an expired or reused ticket, or too many failed attempts |
//! | 4403 | the caller has no role on the project |
//!
//! Failed attempts count against the same per-client limit as the REST API.
//!
//! # Messages
//!
//! The server sends JSON text frames:
//!
//! ```text
//! {"type": "snapshot", "value": <ProjectStatusResponse>}                first, once
//! {"type": "event", "value": <EventResponse>}                         every public event after `?after=`
//! {"type": "delivery", "value": "LIVE" | "QUEUED", "receipt": <GuidanceReceipt>}
//! {"type": "state", "value": <ProjectResponse>}                       after an action
//! {"type": "error", "value": {"code": "...", "message": "..."}}       then the socket closes (1000)
//! ```
//!
//! and accepts (editors and owners; a viewer's message is an error):
//!
//! ```text
//! {"type": "steer", "body": "...", "kind": "STEER" | "ANSWER" | …}    `type` defaults to steer
//! {"type": "interrupt", "body": "..."}                                pauses and interrupts the turn
//! {"type": "action", "action": "start" | "pause" | "resume" | "stop" | "archive"}
//! ```
//!
//! # Delivery
//!
//! Events are read from the durable log after the client's cursor, so a reconnect with
//! `?after=<last sequence>` catches up exactly. The runtime's [`EventNotifier`] wakes the session
//! as soon as this process records an event; a slow poll catches writes from other processes.
//! Each frame is awaited before the next is read from the database, so a slow client is never
//! buffered for: one that cannot take a frame within the send timeout is disconnected. The server
//! pings idle clients and closes (1011) when a ping goes unanswered; on shutdown it closes with
//! 1001.
//!
//! [`EventNotifier`]: crate::app::EventNotifier

use std::collections::BTreeSet;
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Request, State};
use axum::http::HeaderMap;
use axum::http::header::SEC_WEBSOCKET_PROTOCOL;
use axum::response::Response;
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use limina_core::{ErrorKind, LiminaError, Principal, new_uuid};
use limina_domain::collaboration::Role;
use serde::Serialize;
use serde_json::{Map, Value, json};
use tokio::sync::watch;
use tokio::time::{Instant, MissedTickBehavior, interval_at, timeout};

use super::App;
use super::error::not_found;
use super::routes::Params;
use crate::app::{ProjectStatusResponse, public_event, public_status};
use crate::auth::jwks::py_str;

/// The subprotocol every live client must offer.
pub const SUBPROTOCOL: &str = "limina.v1";
/// The prefix of the ticket subprotocol (`limina.ticket.<ticket>`).
pub const TICKET_PREFIX: &str = "limina.ticket.";
/// Events read from the log per database query.
const EVENT_BATCH: i64 = 200;
/// Close reasons are limited to 123 bytes by the protocol; Python cut them at 120 characters.
const REASON_LIMIT: usize = 120;

/// Timing and size limits of live sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveSettings {
    /// How often a session reads the log without a signal (writes by other processes).
    pub poll_interval: Duration,
    /// How long a session stays silent before it pings the client.
    pub ping_interval: Duration,
    /// How long an unanswered ping is tolerated.
    pub pong_timeout: Duration,
    /// How long one frame may take to reach a slow client before it is disconnected.
    pub send_timeout: Duration,
    /// The largest message a client may send.
    pub max_message_bytes: usize,
}

impl Default for LiveSettings {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(1),
            ping_interval: Duration::from_secs(20),
            pong_timeout: Duration::from_secs(20),
            send_timeout: Duration::from_secs(30),
            max_message_bytes: 1024 * 1024,
        }
    }
}

/// Why a connection was refused, sent as its close frame.
struct Refusal {
    code: u16,
    reason: String,
}

impl Refusal {
    fn new(code: u16, reason: impl Into<String>) -> Self {
        Self { code, reason: reason.into() }
    }
}

/// An admitted live client.
struct Admitted {
    principal: Principal,
    role: Role,
    initial: ProjectStatusResponse,
}

/// `/v1/projects/{slug}/live`.
pub async fn live(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> Response {
    let (mut parts, _body) = request.into_parts();
    // Plain HTTP requests find no route here, as with the Python server.
    let Ok(upgrade) = WebSocketUpgrade::from_request_parts(&mut parts, &app).await else {
        return not_found();
    };
    let protocols = requested_protocols(&parts.headers);
    let admitted = admit(&app, &slug, &parts, &protocols).await;
    let settings = app.settings.live;
    let upgrade = upgrade
        .max_message_size(settings.max_message_bytes)
        .max_frame_size(settings.max_message_bytes)
        .write_buffer_size(64 * 1024)
        .max_write_buffer_size(4 * 1024 * 1024);
    let upgrade =
        if protocols.contains(SUBPROTOCOL) { upgrade.protocols([SUBPROTOCOL]) } else { upgrade };
    let after = parts
        .uri
        .query()
        .and_then(|query| {
            url::form_urlencoded::parse(query.as_bytes())
                .filter(|(key, _)| key == "after")
                .map(|(_, value)| value.into_owned())
                .last()
        })
        .map_or(0, |value| python_int(&value).unwrap_or(0).max(0));
    upgrade.on_upgrade(move |socket| async move {
        let (sink, stream) = socket.split();
        let mut session = Session { app, slug, sink, settings };
        match admitted {
            Err(refusal) => session.close(refusal.code, &refusal.reason).await,
            Ok(admitted) => session.run(admitted, stream, after).await,
        }
    })
}

/// The subprotocols the client offered (every `Sec-WebSocket-Protocol` header, comma-separated).
fn requested_protocols(headers: &HeaderMap) -> BTreeSet<String> {
    headers
        .get_all(SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .flat_map(|value| {
            String::from_utf8_lossy(value.as_bytes())
                .split(',')
                .map(|item| item.trim().to_owned())
                .collect::<Vec<_>>()
        })
        .filter(|item| !item.is_empty())
        .collect()
}

/// Authenticates and authorizes a live client the way the Python endpoint did, before the
/// handshake completes (a ticket is consumed even if the client then goes away).
async fn admit(
    app: &App,
    slug: &str,
    parts: &axum::http::request::Parts,
    protocols: &BTreeSet<String>,
) -> Result<Admitted, Refusal> {
    if !protocols.contains(SUBPROTOCOL) {
        return Err(Refusal::new(4400, "The limina.v1 subprotocol is required."));
    }
    let tickets: Vec<&str> =
        protocols.iter().filter_map(|item| item.strip_prefix(TICKET_PREFIX)).collect();
    if tickets.len() > 1 {
        return Err(Refusal::new(4400, "Only one live ticket may be supplied."));
    }
    let ticket = tickets.first().copied().filter(|ticket| !ticket.is_empty());
    let credentials = app.auth.credentials(parts);
    let key = credentials.client_key();
    let limiter = app.auth.client_failures();
    let outcome: limina_core::Result<Admitted> = async {
        limiter.check(&key)?;
        let collaboration = app.runtime.collaboration();
        let (principal, role) = match ticket {
            Some(ticket) => collaboration.consume_live_ticket(slug, ticket).await?,
            None => {
                let principal = crate::auth::Authenticator::authenticate(
                    app.auth.transport(),
                    credentials.bearer_token.as_deref(),
                    credentials.actor_hint.as_deref(),
                )
                .await?;
                let role = collaboration.require_role(slug, &principal, Role::Viewer).await?;
                (principal, role)
            }
        };
        let initial = public_status(&app.runtime.service().status(slug).await?, Some(role));
        Ok(Admitted { principal, role, initial })
    }
    .await;
    match outcome {
        Ok(admitted) => {
            limiter.success(&key);
            Ok(admitted)
        }
        Err(error) => {
            if error.kind == ErrorKind::Authentication || ticket.is_some() {
                limiter.failure(&key);
            }
            let code = if error.kind == ErrorKind::Authorization { 4403 } else { 4401 };
            Err(Refusal::new(code, error.message))
        }
    }
}

/// Python's `int()` of the `after` parameter: whitespace, a sign and digits (with single
/// underscores).
fn python_int(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let digits = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    let valid = !digits.is_empty()
        && !digits.starts_with('_')
        && !digits.ends_with('_')
        && !digits.contains("__")
        && digits.bytes().all(|byte| byte.is_ascii_digit() || byte == b'_');
    if !valid {
        return None;
    }
    let cleaned: String = trimmed.chars().filter(|character| *character != '_').collect();
    match cleaned.parse::<i64>() {
        Ok(value) => Some(value),
        // Beyond i64: a huge positive cursor reads nothing, a huge negative one is 0.
        Err(_) if trimmed.starts_with('-') => Some(0),
        Err(_) => Some(i64::MAX),
    }
}

/// How a session ended early.
struct Gone;

struct Session {
    app: App,
    slug: String,
    sink: SplitSink<WebSocket, Message>,
    settings: LiveSettings,
}

impl Session {
    async fn run(&mut self, admitted: Admitted, mut stream: SplitStream<WebSocket>, after: i64) {
        // Subscribe before the first read so no write in between is missed.
        let mut changes = self.app.runtime.notifier().subscribe(&self.slug);
        let mut shutdown = self.app.shutdown.clone();
        if *shutdown.borrow_and_update() {
            self.close(1001, "The runtime is shutting down.").await;
            return;
        }
        let Admitted { principal, role, initial } = admitted;
        if self.send(&json!({"type": "snapshot", "value": initial})).await.is_err() {
            return;
        }
        let start = Instant::now();
        let mut poll =
            interval_at(start + self.settings.poll_interval, self.settings.poll_interval);
        poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut ping =
            interval_at(start + self.settings.ping_interval, self.settings.ping_interval);
        ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut awaiting_pong: Option<Instant> = None;
        // A server that can no longer signal a shutdown (its sender is gone) never will.
        let mut shutdown_possible = true;
        let mut cursor = after;
        loop {
            match self.push_events(cursor).await {
                Ok(next) => cursor = next,
                Err(Gone) => return,
            }
            tokio::select! {
                changed = shutdown.changed(), if shutdown_possible => {
                    if changed.is_err() {
                        shutdown_possible = false;
                    } else if *shutdown.borrow() {
                        self.close(1001, "The runtime is shutting down.").await;
                        return;
                    }
                }
                message = stream.next() => {
                    awaiting_pong = None;
                    let text = match message {
                        None | Some(Err(_)) => return,
                        Some(Ok(Message::Close(_))) => {
                            let _ = self.sink.close().await;
                            return;
                        }
                        Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                        Some(Ok(Message::Text(text))) => text.to_string(),
                        Some(Ok(Message::Binary(bytes))) => String::from_utf8_lossy(&bytes).into_owned(),
                    };
                    let reply = self.handle(&text, &principal, role).await;
                    let outcome = match reply {
                        Ok(frame) => self.send(&frame).await,
                        Err(error) => {
                            let frame = json!({
                                "type": "error",
                                "value": {"code": error.code(), "message": error.message},
                            });
                            if self.send(&frame).await.is_ok() {
                                self.close(1000, "").await;
                            }
                            return;
                        }
                    };
                    if outcome.is_err() {
                        return;
                    }
                }
                changed = changes.changed() => {
                    if changed.is_err() {
                        self.close(1001, "The runtime is shutting down.").await;
                        return;
                    }
                }
                _ = poll.tick() => {}
                _ = ping.tick() => {
                    match awaiting_pong {
                        Some(since) if since.elapsed() >= self.settings.pong_timeout => {
                            self.close(1011, "keepalive ping timeout").await;
                            return;
                        }
                        Some(_) => {}
                        None => {
                            if self.send_message(Message::Ping(Vec::new().into())).await.is_err() {
                                return;
                            }
                            awaiting_pong = Some(Instant::now());
                        }
                    }
                }
            }
        }
    }

    /// Sends every public event after `cursor`; returns the new cursor (hidden events count).
    async fn push_events(&mut self, mut cursor: i64) -> Result<i64, Gone> {
        loop {
            let events = match self
                .app
                .runtime
                .service()
                .events(&self.slug, cursor, EVENT_BATCH)
                .await
            {
                Ok(events) => events,
                Err(error) => {
                    tracing::warn!(code = error.code(), message = %error.message, "live events unavailable");
                    self.close(1011, "The live event log is unavailable.").await;
                    return Err(Gone);
                }
            };
            let complete = i64::try_from(events.len()).unwrap_or(i64::MAX) < EVENT_BATCH;
            for event in &events {
                cursor = i64::from(event.sequence);
                if let Some(public) = public_event(event) {
                    self.send(&json!({"type": "event", "value": public})).await?;
                }
            }
            if complete {
                return Ok(cursor);
            }
        }
    }

    /// Answers one client message with the frame to send, or the error that ends the session.
    async fn handle(
        &self,
        text: &str,
        principal: &Principal,
        role: Role,
    ) -> limina_core::Result<Value> {
        let message: Map<String, Value> = match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(message)) => message,
            _ => {
                return Err(LiminaError::invariant("Live messages must be JSON objects.")
                    .with_code("invalid_request"));
            }
        };
        let text_of =
            |key: &str, default: &str| message.get(key).map_or_else(|| default.to_owned(), py_str);
        let message_type = text_of("type", "steer").to_lowercase();
        if role == Role::Viewer {
            return Err(LiminaError::invariant(
                "Viewers can observe live work but cannot steer it.",
            ));
        }
        let operations = self.app.runtime.operations();
        match message_type.as_str() {
            "steer" => {
                let body = text_of("body", "");
                let body = body.trim();
                if body.is_empty() {
                    return Err(LiminaError::invariant("Steering message cannot be empty."));
                }
                let kind = text_of("kind", "STEER");
                let receipt =
                    operations.steer(&self.slug, body, &kind, &new_uuid(), principal).await?;
                Ok(json!({"type": "delivery", "value": receipt.delivery, "receipt": receipt}))
            }
            "interrupt" => {
                let body = text_of("body", "Pause and await human direction.");
                let receipt = operations
                    .steer(&self.slug, &body, "INTERRUPT", &new_uuid(), principal)
                    .await?;
                Ok(json!({"type": "delivery", "value": receipt.delivery, "receipt": receipt}))
            }
            "action" => {
                let action = text_of("action", "").to_lowercase();
                let project =
                    operations.apply_lifecycle(&self.slug, &action, &new_uuid(), principal).await?;
                Ok(json!({"type": "state", "value": project}))
            }
            _ => Err(LiminaError::invariant("Live messages must be steer, interrupt, or action.")
                .with("message_type", message_type)),
        }
    }

    async fn send<T: Serialize>(&mut self, frame: &T) -> Result<(), Gone> {
        let text = serde_json::to_string(frame).map_err(|_| Gone)?;
        self.send_message(Message::Text(text.into())).await
    }

    /// Sends one frame, or gives up on a client that cannot take it in time.
    async fn send_message(&mut self, message: Message) -> Result<(), Gone> {
        match timeout(self.settings.send_timeout, self.sink.send(message)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(Gone),
            Err(_) => {
                tracing::info!(project = %self.slug, "disconnecting a live client that stopped reading");
                Err(Gone)
            }
        }
    }

    async fn close(&mut self, code: u16, reason: &str) {
        let frame = CloseFrame { code, reason: close_reason(reason).into() };
        let _ = self.send_message(Message::Close(Some(frame))).await;
    }
}

/// At most 120 characters and 123 bytes.
fn close_reason(reason: &str) -> String {
    let mut text: String = reason.chars().take(REASON_LIMIT).collect();
    while text.len() > 123 {
        text.pop();
    }
    text
}

/// Signals live sessions that the server is stopping.
pub fn shutdown_signal() -> (watch::Sender<bool>, watch::Receiver<bool>) {
    watch::channel(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_parse_like_python_int() {
        assert_eq!(python_int("12"), Some(12));
        assert_eq!(python_int(" +7 "), Some(7));
        assert_eq!(python_int("-3"), Some(-3));
        assert_eq!(python_int("1_0"), Some(10));
        assert_eq!(python_int("x"), None);
        assert_eq!(python_int(""), None);
        assert_eq!(python_int("1.5"), None);
        assert_eq!(python_int("99999999999999999999"), Some(i64::MAX));
    }

    #[test]
    fn close_reasons_fit_the_protocol() {
        assert_eq!(close_reason("short"), "short");
        assert_eq!(close_reason(&"a".repeat(200)).len(), 120);
        assert!(close_reason(&"é".repeat(100)).len() <= 123);
    }

    #[test]
    fn offered_protocols_are_split_and_trimmed() {
        let mut headers = HeaderMap::new();
        headers.append(SEC_WEBSOCKET_PROTOCOL, "limina.v1, limina.ticket.abc".parse().unwrap());
        headers.append(SEC_WEBSOCKET_PROTOCOL, "other".parse().unwrap());
        let protocols = requested_protocols(&headers);
        assert!(protocols.contains("limina.v1"));
        assert!(protocols.contains("limina.ticket.abc"));
        assert!(protocols.contains("other"));
    }
}
