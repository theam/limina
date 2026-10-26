//! `limina attach`: the live project over WebSocket. Activity streams in while the user types
//! guidance or slash commands.

use futures::{SinkExt, StreamExt};
use limina_core::LiminaError;
use reqwest::Url;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

use super::project::{render_activity, render_project};
use super::{Context, Failure, Outcome};
use crate::output::{self, display, field, panel, title_case};
use crate::pyjson;

const SUBPROTOCOL: &str = "limina.v1";
const HELP: &str = "Type feedback, or use /pause, /resume, /stop, /interrupt, /detach.";

pub async fn attach(context: &Context, project: &str) -> Outcome {
    if context.out.as_json {
        return Err(Failure::Usage {
            command: &["attach"],
            message: "Invalid value: attach is interactive and cannot be combined with --json"
                .to_owned(),
        });
    }
    session(context, project).await.map_err(|reason| {
        Failure::Limina(
            LiminaError::invariant("Could not attach to the live project.")
                .with("reason", reason)
                .with("project", project),
        )
    })
}

/// The live endpoint: the instance URL with a WebSocket scheme.
pub fn live_url(base: &str, project: &str) -> Result<Url, String> {
    let base = base.trim_end_matches('/');
    let base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_owned()
    };
    let mut url = Url::parse(&base).map_err(|error| error.to_string())?;
    url.path_segments_mut()
        .map_err(|()| "the instance URL cannot be a base".to_owned())?
        .pop_if_empty()
        .extend(["v1", "projects", project, "live"]);
    Ok(url)
}

/// One message from the server, rendered for the terminal.
fn render_message(raw: &str) {
    let Ok(message) = serde_json::from_str::<Value>(raw) else {
        return;
    };
    let empty = Value::Null;
    let value = message.get("value").unwrap_or(&empty);
    match message.get("type").and_then(Value::as_str) {
        Some("event") => output::line(&render_activity(value)),
        Some("snapshot") => {
            output::line(value.get("project").map(render_project).unwrap_or_default().trim_end());
        }
        Some("delivery") => {
            output::line(&format!("{} guidance accepted", title_case(&display(value))));
        }
        Some("state") => output::line(render_project(value).trim_end()),
        Some("error") => output::error_line(&field(value, "message")),
        _ => {}
    }
}

/// The live message a typed line sends.
fn outgoing(line: &str) -> Value {
    match line {
        "/pause" | "/resume" | "/stop" => {
            json!({"type": "action", "action": line.trim_start_matches('/')})
        }
        "/interrupt" => json!({"type": "interrupt"}),
        _ => json!({"type": "steer", "body": line}),
    }
}

fn prompt() {
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    let _ = write!(stdout, "limina> ");
    let _ = stdout.flush();
}

async fn session(context: &Context, project: &str) -> Result<(), String> {
    let url = live_url(&context.url, project)?;
    let mut request = url.as_str().into_client_request().map_err(|error| error.to_string())?;
    let headers = request.headers_mut();
    headers.insert(
        "x-limina-actor",
        HeaderValue::from_bytes(context.actor.as_bytes()).map_err(|error| error.to_string())?,
    );
    if let Some(token) = context.token.as_deref().filter(|token| !token.is_empty()) {
        let mut value =
            HeaderValue::from_str(&format!("Bearer {token}")).map_err(|error| error.to_string())?;
        value.set_sensitive(true);
        headers.insert("authorization", value);
    }
    headers.insert("sec-websocket-protocol", HeaderValue::from_static(SUBPROTOCOL));
    let (socket, _response) =
        tokio_tungstenite::connect_async(request).await.map_err(|error| error.to_string())?;
    let (mut sender, mut receiver) = socket.split();
    output::line(
        panel(
            &format!("Attached to {project}"),
            "You are inside the live project. Type feedback directly.\n\
             Commands: /pause  /resume  /stop  /interrupt  /detach  /help",
        )
        .trim_end(),
    );

    let mut reader = tokio::spawn(async move {
        while let Some(message) = receiver.next().await {
            match message {
                Ok(Message::Text(text)) => render_message(text.as_str()),
                Ok(Message::Binary(bytes)) => render_message(&String::from_utf8_lossy(&bytes)),
                Ok(Message::Close(_)) => return None,
                Ok(_) => {}
                Err(error) => return Some(error.to_string()),
            }
        }
        None
    });

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let result = loop {
        prompt();
        let line = tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(line)) => line,
                // End of input detaches, like `/detach`.
                Ok(None) | Err(_) => break Ok(()),
            },
            closed = &mut reader => {
                output::line("");
                if let Ok(Some(reason)) = closed {
                    output::error_line(&format!("Live connection closed: {reason}"));
                } else {
                    output::error_line("Live connection closed.");
                }
                return Ok(());
            }
            _ = tokio::signal::ctrl_c() => break Ok(()),
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "/detach" {
            break Ok(());
        }
        if line == "/help" {
            output::line(HELP);
            continue;
        }
        let message = pyjson::dumps(&outgoing(line));
        if let Err(error) = sender.send(Message::text(message)).await {
            break Err(error.to_string());
        }
    };
    let _ = sender.send(Message::Close(None)).await;
    reader.abort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_urls_switch_to_websocket_schemes() {
        assert_eq!(
            live_url("http://127.0.0.1:7433/", "demo").unwrap().as_str(),
            "ws://127.0.0.1:7433/v1/projects/demo/live"
        );
        assert_eq!(
            live_url("https://limina.example/base", "a b").unwrap().as_str(),
            "wss://limina.example/base/v1/projects/a%20b/live"
        );
    }

    #[test]
    fn typed_lines_become_live_messages() {
        assert_eq!(outgoing("/pause"), json!({"type": "action", "action": "pause"}));
        assert_eq!(outgoing("/interrupt"), json!({"type": "interrupt"}));
        assert_eq!(outgoing("Try B"), json!({"type": "steer", "body": "Try B"}));
    }
}
