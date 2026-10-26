//! Normalizes Codex and Claude streams into conversation-grade runtime events.
//!
//! Every provider message becomes a [`RuntimeEvent`] whose `detail` follows one schema, so the web
//! console can render the work as a transcript without knowing which engine produced it:
//!
//! - `kind`: `message` | `tool` | `plan` | `status` | `knowledge`
//! - `text`: full agent markdown for messages and plans, a short line for status rows
//! - `tool` / `input`: display name and a one-line summary of the call's input
//! - `item_id` / `parent_item_id`: stable ids that merge started and completed rows and nest
//!   subagents
//! - `phase` / `state` / `exit_code` / `duration_ms` / `files`: tool lifecycle facts
//!
//! Tool results, command output, reasoning and file diffs are never included. Internal Limina
//! knowledge commands are reported as `kind = "knowledge"` without their command line.
//!
//! The mappers read the providers' wire JSON directly: Codex `app-server` notifications
//! (`method` + `params`) and the Claude CLI's `stream-json` messages. The `item_type` values are
//! the class names the official Python SDKs gave those messages, because they are persisted.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::{NoExpand, Regex};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::text::{clip, non_empty, one_line, py_str, splitlines, strict_int, strip, truthy};

pub const CLAUDE_EVENT: &str = "runtime.claude-code";
pub const CODEX_EVENT: &str = "runtime.codex";
pub const TEXT_LIMIT: usize = 16_000;
pub const INPUT_LIMIT: usize = 500;
pub const SUMMARY_LIMIT: usize = 200;
pub const KNOWLEDGE_MARKER: &str = "limina _agent";
pub const KNOWLEDGE_SUMMARY: &str = "Updating durable project knowledge";
pub const KNOWLEDGE_TOOL: &str = "limina";
/// The structured checkpoint every turn must return. The supervisor records it as
/// `runtime.turn_completed` and `project.checkpoint`, so the transcript mappers hide its raw form.
pub const CHECKPOINT_FIELDS: [&str; 5] =
    ["summary", "status", "current_objective", "next_step", "blocker"];
/// Claude satisfies the JSON-schema output format by calling this tool with the checkpoint.
const CLAUDE_HIDDEN_TOOLS: &[&str] = &["StructuredOutput"];
const CODEX_TOOL_ITEMS: &[&str] =
    &["CommandExecution", "McpToolCall", "WebSearch", "CollabAgentToolCall", "DynamicToolCall"];
const INTERNAL_COMMAND_LABEL: &str = "[internal Limina knowledge command]";

#[allow(clippy::expect_used, reason = "static patterns, exercised by the tests")]
static SHELL_WRAPPERS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    // Python used one pattern with a back-reference to the opening quote; the regex crate has no
    // back-references, so each quote style gets its own pattern.
    ["'", "\""].map(|quote| {
        Regex::new(&format!(r"(?s)^(?:\S*/)?(?:ba|z)?sh\s+-l?c\s+{quote}(?P<body>.*){quote}$"))
            .expect("valid shell wrapper pattern")
    })
});

#[allow(clippy::expect_used, reason = "static pattern, exercised by the tests")]
static INTERNAL_COMMAND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"limina _agent(?:\\\n|[^\n`])*").expect("valid internal command pattern")
});

/// One normalized provider event. `detail` keeps the Python key order, so stored payloads are
/// byte-for-byte the shape the console already renders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeEvent {
    pub event_type: String,
    pub summary: String,
    pub detail: Map<String, Value>,
}

impl RuntimeEvent {
    pub fn new(
        event_type: impl Into<String>,
        summary: impl Into<String>,
        detail: Map<String, Value>,
    ) -> Self {
        Self { event_type: event_type.into(), summary: summary.into(), detail }
    }

    /// The persisted payload: `{"summary": …, **detail}` (a `summary` key in `detail` wins).
    pub fn payload(&self) -> Map<String, Value> {
        let mut payload = Map::new();
        payload.insert("summary".to_owned(), Value::String(self.summary.clone()));
        payload.extend(self.detail.clone());
        payload
    }

    /// A `detail` string field, if present.
    pub fn detail_str(&self, key: &str) -> Option<&str> {
        self.detail.get(key).and_then(Value::as_str)
    }
}

/// True for strings that contain an internal `limina _agent` command.
pub fn contains_internal_command(text: &str) -> bool {
    text.contains(KNOWLEDGE_MARKER)
}

/// `is_internal_command(value)`: a JSON string that contains an internal command.
pub fn is_internal_command(value: &Value) -> bool {
    value.as_str().is_some_and(contains_internal_command)
}

/// Hides internal command lines (and their arguments) inside otherwise public prose.
pub fn redact_internal_commands(text: &str) -> String {
    INTERNAL_COMMAND.replace_all(text, NoExpand(INTERNAL_COMMAND_LABEL)).into_owned()
}

/// Counts each tool once: on start, or on completion for Codex edits that never start. The
/// supervisor uses this for a run's tool-call counter.
pub fn is_tool_call(detail: &Map<String, Value>) -> bool {
    let kind = detail.get("kind").and_then(Value::as_str);
    let phase = detail.get("phase").and_then(Value::as_str);
    if matches!(kind, Some("tool" | "knowledge")) && phase == Some("started") {
        return true;
    }
    kind == Some("tool")
        && phase == Some("completed")
        && detail.get("tool").and_then(Value::as_str) == Some("edit")
}

fn first_line(text: &str) -> String {
    splitlines(text)
        .into_iter()
        .find(|line| !strip(line).is_empty())
        .map(|line| one_line(line, SUMMARY_LIMIT))
        .unwrap_or_default()
}

fn first_short_string(values: &Map<String, Value>) -> Option<String> {
    values
        .values()
        .find_map(|value| non_empty(Some(value)).filter(|text| text.chars().count() <= INPUT_LIMIT))
}

fn arguments_summary(arguments: Option<&Value>) -> Option<String> {
    let arguments = arguments.unwrap_or(&Value::Null);
    match arguments {
        Value::String(_) => return non_empty(Some(arguments)),
        Value::Object(values) => {
            if let Some(text) = first_short_string(values) {
                return Some(text);
            }
        }
        _ => {}
    }
    let empty = match arguments {
        Value::Null => true,
        Value::Object(values) => values.is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    };
    if empty { None } else { serde_json::to_string(arguments).ok() }
}

fn is_checkpoint(text: &str) -> bool {
    let body = strip(text);
    if !body.starts_with('{') {
        return false;
    }
    match serde_json::from_str::<Value>(body) {
        Ok(Value::Object(value)) => CHECKPOINT_FIELDS.iter().all(|key| value.contains_key(*key)),
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileChange {
    path: String,
    kind: String,
}

/// The `detail` schema, in its persisted key order.
#[derive(Default)]
struct Detail {
    method: String,
    item_type: String,
    kind: &'static str,
    phase: &'static str,
    text: Option<String>,
    tool: Option<String>,
    input: Option<String>,
    item_id: Option<String>,
    parent_item_id: Option<String>,
    state: Option<&'static str>,
    exit_code: Option<i64>,
    duration_ms: Option<i64>,
    files: Option<Vec<FileChange>>,
}

impl Detail {
    fn into_map(self) -> Map<String, Value> {
        fn opt<T: Into<Value>>(value: Option<T>) -> Value {
            value.map_or(Value::Null, Into::into)
        }
        let files = self.files.map(|files| {
            Value::Array(
                files
                    .into_iter()
                    .map(|file| {
                        let mut entry = Map::new();
                        entry.insert("path".to_owned(), Value::String(file.path));
                        entry.insert("kind".to_owned(), Value::String(file.kind));
                        Value::Object(entry)
                    })
                    .collect(),
            )
        });
        let mut detail = Map::new();
        detail.insert("method".to_owned(), Value::String(self.method));
        detail.insert("item_type".to_owned(), Value::String(self.item_type));
        detail.insert("kind".to_owned(), Value::String(self.kind.to_owned()));
        detail.insert("text".to_owned(), opt(self.text));
        detail.insert("tool".to_owned(), opt(self.tool));
        detail.insert("input".to_owned(), opt(self.input));
        detail.insert("item_id".to_owned(), opt(self.item_id));
        detail.insert("parent_item_id".to_owned(), opt(self.parent_item_id));
        detail.insert("phase".to_owned(), Value::String(self.phase.to_owned()));
        detail.insert("state".to_owned(), opt(self.state));
        detail.insert("exit_code".to_owned(), opt(self.exit_code));
        detail.insert("duration_ms".to_owned(), opt(self.duration_ms));
        detail.insert("files".to_owned(), files.unwrap_or(Value::Null));
        detail
    }
}

struct Prose<'a> {
    event_type: &'static str,
    method: &'a str,
    item_type: &'a str,
    text: &'a str,
    kind: &'static str,
    item_id: Option<String>,
    parent_item_id: Option<String>,
}

fn prose_event(prose: Prose<'_>) -> Option<RuntimeEvent> {
    let body = strip(prose.text);
    if body.is_empty() {
        return None;
    }
    let detail = Detail {
        method: prose.method.to_owned(),
        item_type: prose.item_type.to_owned(),
        kind: prose.kind,
        phase: "completed",
        text: Some(clip(body, TEXT_LIMIT)),
        item_id: prose.item_id,
        parent_item_id: prose.parent_item_id,
        ..Detail::default()
    };
    Some(RuntimeEvent::new(prose.event_type, first_line(body), detail.into_map()))
}

struct Status<'a> {
    event_type: &'static str,
    method: &'a str,
    item_type: &'a str,
    text: &'a str,
    phase: &'static str,
    state: Option<&'static str>,
    item_id: Option<String>,
    parent_item_id: Option<String>,
}

fn status_event(status: Status<'_>) -> RuntimeEvent {
    let line = one_line(status.text, INPUT_LIMIT);
    let summary = first_line(&line);
    let detail = Detail {
        method: status.method.to_owned(),
        item_type: status.item_type.to_owned(),
        kind: "status",
        phase: status.phase,
        text: Some(line),
        item_id: status.item_id,
        parent_item_id: status.parent_item_id,
        state: status.state,
        ..Detail::default()
    };
    RuntimeEvent::new(status.event_type, summary, detail.into_map())
}

fn tool_summary(tool: &str, phase: &str, state: Option<&str>) -> String {
    if phase == "started" {
        return format!("Using {tool}");
    }
    let outcome = match state {
        Some("failed") => "failed",
        Some("declined") => "was declined",
        _ => "finished",
    };
    format!("{tool} {outcome}")
}

struct Tool<'a> {
    event_type: &'static str,
    method: &'a str,
    item_type: &'a str,
    tool: String,
    raw_input: Option<String>,
    item_id: Option<String>,
    phase: &'static str,
    state: Option<&'static str>,
    internal: bool,
    parent_item_id: Option<String>,
    exit_code: Option<i64>,
    duration_ms: Option<i64>,
    files: Option<Vec<FileChange>>,
    summary: Option<String>,
}

fn tool_event(tool: Tool<'_>) -> RuntimeEvent {
    let internal =
        tool.internal || tool.raw_input.as_deref().is_some_and(contains_internal_command);
    if internal {
        let detail = Detail {
            method: tool.method.to_owned(),
            item_type: tool.item_type.to_owned(),
            kind: "knowledge",
            phase: tool.phase,
            tool: Some(KNOWLEDGE_TOOL.to_owned()),
            item_id: tool.item_id,
            parent_item_id: tool.parent_item_id,
            state: tool.state,
            exit_code: tool.exit_code,
            duration_ms: tool.duration_ms,
            ..Detail::default()
        };
        return RuntimeEvent::new(tool.event_type, KNOWLEDGE_SUMMARY, detail.into_map());
    }
    let summary = tool.summary.unwrap_or_else(|| tool_summary(&tool.tool, tool.phase, tool.state));
    let detail = Detail {
        method: tool.method.to_owned(),
        item_type: tool.item_type.to_owned(),
        kind: "tool",
        phase: tool.phase,
        tool: Some(tool.tool),
        input: tool.raw_input.filter(|raw| !raw.is_empty()).map(|raw| one_line(&raw, INPUT_LIMIT)),
        item_id: tool.item_id,
        parent_item_id: tool.parent_item_id,
        state: tool.state,
        exit_code: tool.exit_code,
        duration_ms: tool.duration_ms,
        files: tool.files,
        ..Detail::default()
    };
    RuntimeEvent::new(tool.event_type, summary, detail.into_map())
}

// Claude ----------------------------------------------------------------------------------------

const HIDDEN: (&str, &str) = ("hidden", "");

/// Remembers each `tool_use` id within a turn, so result rows carry the same display name and
/// kind as the call they complete (and hidden calls stay hidden).
#[derive(Debug, Default, Clone)]
pub struct ClaudeToolNames {
    calls: HashMap<String, (String, String)>,
}

impl ClaudeToolNames {
    pub fn new() -> Self {
        Self::default()
    }

    fn remember(&mut self, item_id: &str, kind: &str, tool: &str) {
        self.calls.insert(item_id.to_owned(), (kind.to_owned(), tool.to_owned()));
    }

    fn lookup(&self, item_id: &str) -> (String, String) {
        self.calls.get(item_id).cloned().unwrap_or_else(|| ("tool".to_owned(), "tool".to_owned()))
    }
}

fn claude_tool_name(name: &str) -> String {
    if name.starts_with("mcp__") {
        let parts: Vec<&str> = name.splitn(3, "__").collect();
        if let [_, server, tool] = parts.as_slice()
            && !server.is_empty()
            && !tool.is_empty()
        {
            return format!("mcp:{server}/{tool}");
        }
    }
    name.to_owned()
}

fn claude_input_keys(name: &str) -> &'static [&'static str] {
    match name {
        "Bash" => &["command"],
        "Read" | "Write" | "Edit" | "MultiEdit" => &["file_path"],
        "NotebookEdit" => &["notebook_path", "file_path"],
        "WebFetch" | "web_fetch" => &["url"],
        "WebSearch" | "web_search" => &["query"],
        "Task" | "Agent" => &["description"],
        _ => &[],
    }
}

fn claude_tool_input(name: &str, values: &Map<String, Value>) -> Option<String> {
    if matches!(name, "Glob" | "Grep") {
        let pattern = non_empty(values.get("pattern"));
        let path = non_empty(values.get("path"));
        return match (pattern, path) {
            (Some(pattern), Some(path)) => Some(format!("{pattern} in {path}")),
            (pattern, path) => pattern.or(path),
        };
    }
    if name == "TodoWrite" {
        let count = values.get("todos").and_then(Value::as_array).map_or(0, Vec::len);
        return Some(if count == 1 { "1 todo".to_owned() } else { format!("{count} todos") });
    }
    claude_input_keys(name)
        .iter()
        .find_map(|key| non_empty(values.get(*key)))
        .or_else(|| first_short_string(values))
}

/// `str(getattr(value, name, "") or default)`.
fn text_or(value: Option<&Value>, default: &str) -> String {
    match value {
        Some(value) if truthy(value) => py_str(value),
        _ => default.to_owned(),
    }
}

fn claude_tool_use(
    block: &Value,
    item_type: &str,
    tools: &mut ClaudeToolNames,
    parent: Option<String>,
) -> Option<RuntimeEvent> {
    let name = text_or(block.get("name"), "tool");
    let item_id = non_empty(block.get("id"));
    if CLAUDE_HIDDEN_TOOLS.contains(&name.as_str()) {
        if let Some(item_id) = &item_id {
            tools.remember(item_id, HIDDEN.0, HIDDEN.1);
        }
        return None;
    }
    let empty = Map::new();
    let values = block.get("input").and_then(Value::as_object).unwrap_or(&empty);
    let event = tool_event(Tool {
        event_type: CLAUDE_EVENT,
        method: "tool/use",
        item_type,
        tool: claude_tool_name(&name),
        raw_input: claude_tool_input(&name, values),
        item_id: item_id.clone(),
        phase: "started",
        state: Some("running"),
        internal: false,
        parent_item_id: parent,
        exit_code: None,
        duration_ms: None,
        files: None,
        summary: None,
    });
    if let Some(item_id) = &item_id {
        let kind = event.detail_str("kind").unwrap_or("tool").to_owned();
        let tool = event.detail_str("tool").unwrap_or("tool").to_owned();
        tools.remember(item_id, &kind, &tool);
    }
    Some(event)
}

fn claude_tool_result(
    block: &Value,
    item_type: &str,
    tools: &ClaudeToolNames,
    parent: Option<String>,
) -> Option<RuntimeEvent> {
    let item_id = non_empty(block.get("tool_use_id"));
    let (kind, tool) = tools.lookup(item_id.as_deref().unwrap_or(""));
    if (kind.as_str(), tool.as_str()) == HIDDEN {
        return None;
    }
    let failed = if item_type == "ServerToolResultBlock" {
        block
            .get("content")
            .and_then(Value::as_object)
            .is_some_and(|content| text_or(content.get("type"), "").ends_with("_error"))
    } else {
        block.get("is_error").is_some_and(truthy)
    };
    Some(tool_event(Tool {
        event_type: CLAUDE_EVENT,
        method: "tool/result",
        item_type,
        tool,
        raw_input: None,
        item_id,
        phase: "completed",
        state: Some(if failed { "failed" } else { "ok" }),
        internal: kind == "knowledge",
        parent_item_id: parent,
        exit_code: None,
        duration_ms: None,
        files: None,
        summary: None,
    }))
}

/// The SDK class a content block parses into, by message role. Blocks the SDK skipped stay
/// skipped.
fn claude_block_class(block_type: &str, assistant: bool) -> Option<&'static str> {
    match (block_type, assistant) {
        ("text", _) => Some("TextBlock"),
        ("thinking", true) => Some("ThinkingBlock"),
        ("tool_use", _) => Some("ToolUseBlock"),
        ("tool_result", _) => Some("ToolResultBlock"),
        ("server_tool_use", true) => Some("ServerToolUseBlock"),
        ("advisor_tool_result", true) => Some("ServerToolResultBlock"),
        _ => None,
    }
}

fn claude_block(
    block: &Value,
    tools: &mut ClaudeToolNames,
    assistant: bool,
    parent: Option<String>,
) -> Option<RuntimeEvent> {
    let class = claude_block_class(block.get("type")?.as_str()?, assistant)?;
    match class {
        "TextBlock" if assistant => {
            let text = text_or(block.get("text"), "");
            if is_checkpoint(&text) {
                return None;
            }
            prose_event(Prose {
                event_type: CLAUDE_EVENT,
                method: "assistant/message",
                item_type: class,
                text: &text,
                kind: "message",
                item_id: None,
                parent_item_id: parent,
            })
        }
        "ToolUseBlock" | "ServerToolUseBlock" => claude_tool_use(block, class, tools, parent),
        "ToolResultBlock" | "ServerToolResultBlock" => {
            claude_tool_result(block, class, tools, parent)
        }
        _ => None,
    }
}

fn claude_task(message: &Value, class: &str) -> RuntimeEvent {
    let (text, phase, state) = if class == "TaskNotificationMessage" {
        let state = match text_or(message.get("status"), "").as_str() {
            "completed" => Some("ok"),
            "failed" => Some("failed"),
            _ => None,
        };
        (text_or(message.get("summary"), ""), "completed", state)
    } else {
        (text_or(message.get("description"), ""), "started", Some("running"))
    };
    let stripped = strip(&text);
    status_event(Status {
        event_type: CLAUDE_EVENT,
        method: "task/progress",
        item_type: class,
        text: if stripped.is_empty() { "Subagent task update" } else { stripped },
        phase,
        state,
        item_id: non_empty(message.get("task_id")),
        parent_item_id: non_empty(message.get("tool_use_id")),
    })
}

/// Maps one Claude CLI `stream-json` message to zero or more transcript events.
pub fn map_claude_message(message: &Value, tools: &mut ClaudeToolNames) -> Vec<RuntimeEvent> {
    let message_type = message.get("type").and_then(Value::as_str);
    if message_type == Some("system") {
        let class = match message.get("subtype").and_then(Value::as_str) {
            Some("task_started") => "TaskStartedMessage",
            Some("task_progress") => "TaskProgressMessage",
            Some("task_notification") => "TaskNotificationMessage",
            Some("compact_boundary") => {
                return vec![status_event(Status {
                    event_type: CLAUDE_EVENT,
                    method: "system/compact",
                    item_type: "SystemMessage",
                    text: "Context compacted",
                    phase: "completed",
                    state: None,
                    item_id: None,
                    parent_item_id: None,
                })];
            }
            _ => return Vec::new(),
        };
        return vec![claude_task(message, class)];
    }
    let assistant = match message_type {
        Some("assistant") => true,
        Some("user") => false,
        _ => return Vec::new(),
    };
    let Some(content) =
        message.get("message").and_then(|body| body.get("content")).and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let parent = non_empty(message.get("parent_tool_use_id"));
    content
        .iter()
        .filter_map(|block| claude_block(block, tools, assistant, parent.clone()))
        .collect()
}

// Codex -----------------------------------------------------------------------------------------

fn codex_item_type(item: &Map<String, Value>) -> String {
    let raw = item.get("type").and_then(Value::as_str).unwrap_or("");
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn codex_state(item: &Map<String, Value>, phase: &str) -> Option<&'static str> {
    if phase == "started" {
        return Some("running");
    }
    let state = match item.get("status").and_then(Value::as_str) {
        Some("inProgress") => Some("running"),
        Some("completed") => Some("ok"),
        Some("failed") => Some("failed"),
        Some("declined") => Some("declined"),
        _ => None,
    };
    if item.get("success") == Some(&Value::Bool(false)) {
        return Some("failed");
    }
    state.or_else(|| match strict_int(item.get("exitCode")) {
        None | Some(0) => Some("ok"),
        Some(_) => Some("failed"),
    })
}

fn unwrap_shell(command: &str) -> String {
    let stripped = strip(command);
    SHELL_WRAPPERS
        .iter()
        .find_map(|pattern| pattern.captures(stripped))
        .and_then(|captures| captures.name("body"))
        .map_or_else(|| command.to_owned(), |body| body.as_str().to_owned())
}

fn codex_tool_identity(item: &Map<String, Value>, item_type: &str) -> (String, Option<String>) {
    match item_type {
        "CommandExecution" => {
            ("command".to_owned(), non_empty(item.get("command")).map(|c| unwrap_shell(&c)))
        }
        "McpToolCall" => {
            let server = non_empty(item.get("server")).unwrap_or_else(|| "mcp".to_owned());
            let tool = non_empty(item.get("tool")).unwrap_or_else(|| "tool".to_owned());
            (format!("mcp:{server}/{tool}"), arguments_summary(item.get("arguments")))
        }
        "DynamicToolCall" => {
            let tool = non_empty(item.get("tool")).unwrap_or_else(|| "tool".to_owned());
            let name = match non_empty(item.get("namespace")) {
                Some(namespace) => format!("{namespace}/{tool}"),
                None => tool,
            };
            (name, arguments_summary(item.get("arguments")))
        }
        "WebSearch" => {
            let action = item.get("action");
            let query = non_empty(item.get("query"))
                .or_else(|| non_empty(action.and_then(|a| a.get("query"))))
                .or_else(|| non_empty(action.and_then(|a| a.get("url"))));
            ("web_search".to_owned(), query)
        }
        _ => {
            let input = non_empty(item.get("prompt")).or_else(|| non_empty(item.get("tool")));
            ("subagent".to_owned(), input)
        }
    }
}

fn codex_tool(
    item: &Map<String, Value>,
    item_type: &str,
    method: &str,
    phase: &'static str,
) -> RuntimeEvent {
    let (tool, raw_input) = codex_tool_identity(item, item_type);
    tool_event(Tool {
        event_type: CODEX_EVENT,
        method,
        item_type,
        tool,
        raw_input,
        item_id: non_empty(item.get("id")),
        phase,
        state: codex_state(item, phase),
        internal: false,
        parent_item_id: None,
        exit_code: strict_int(item.get("exitCode")),
        duration_ms: strict_int(item.get("durationMs")),
        files: None,
        summary: None,
    })
}

fn change_kind(kind: Option<&Value>) -> String {
    let raw = match kind {
        Some(Value::String(text)) => Some(Value::String(text.clone())),
        Some(Value::Object(object)) => object.get("type").cloned(),
        _ => None,
    };
    raw.filter(truthy).map_or_else(|| "update".to_owned(), |value| py_str(&value))
}

fn codex_file_change(item: &Map<String, Value>, method: &str) -> RuntimeEvent {
    let files: Vec<FileChange> = item
        .get("changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|change| {
            let path = non_empty(change.get("path"))?;
            Some(FileChange { path, kind: change_kind(change.get("kind")) })
        })
        .collect();
    let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
    let summary = match paths.as_slice() {
        [single] => format!("Edited {single}"),
        _ => format!("Edited {} files", paths.len()),
    };
    let raw_input = Some(paths.join(", ")).filter(|joined| !joined.is_empty());
    tool_event(Tool {
        event_type: CODEX_EVENT,
        method,
        item_type: "FileChange",
        tool: "edit".to_owned(),
        raw_input,
        item_id: non_empty(item.get("id")),
        phase: "completed",
        state: codex_state(item, "completed"),
        internal: false,
        parent_item_id: None,
        exit_code: None,
        duration_ms: None,
        files: Some(files),
        summary: Some(one_line(&summary, SUMMARY_LIMIT)),
    })
}

fn codex_completed(
    item: &Map<String, Value>,
    item_type: &str,
    method: &str,
) -> Option<RuntimeEvent> {
    let item_id = non_empty(item.get("id"));
    if CODEX_TOOL_ITEMS.contains(&item_type) {
        return Some(codex_tool(item, item_type, method, "completed"));
    }
    match item_type {
        "FileChange" => Some(codex_file_change(item, method)),
        "AgentMessage" | "Plan" => {
            let text = text_or(item.get("text"), "");
            if item_type == "AgentMessage" && is_checkpoint(&text) {
                return None;
            }
            prose_event(Prose {
                event_type: CODEX_EVENT,
                method,
                item_type,
                text: &text,
                kind: if item_type == "AgentMessage" { "message" } else { "plan" },
                item_id,
                parent_item_id: None,
            })
        }
        "ContextCompaction" => Some(status_event(Status {
            event_type: CODEX_EVENT,
            method,
            item_type,
            text: "Context compacted",
            phase: "completed",
            state: None,
            item_id,
            parent_item_id: None,
        })),
        _ => None,
    }
}

fn codex_error(params: &Value) -> RuntimeEvent {
    let mut message = non_empty(params.get("error").and_then(|error| error.get("message")))
        .unwrap_or_else(|| "Codex reported an error.".to_owned());
    if params.get("willRetry").is_some_and(truthy) {
        message = format!("{}. Codex will retry.", message.trim_end_matches('.'));
    }
    status_event(Status {
        event_type: CODEX_EVENT,
        method: "error",
        item_type: "Error",
        text: &message,
        phase: "completed",
        state: Some("failed"),
        item_id: None,
        parent_item_id: None,
    })
}

/// Maps one Codex `app-server` notification to at most one transcript event.
///
/// Turn start and completion are recorded by the supervisor. Item starts are only reported for
/// tools, because messages, plans and file changes are complete (and meaningful) only on
/// completion.
pub fn map_codex_notification(method: &str, params: &Value) -> Option<RuntimeEvent> {
    if method == "error" {
        return Some(codex_error(params));
    }
    if method != "item/started" && method != "item/completed" {
        return None;
    }
    let item = params.get("item")?.as_object()?;
    let item_type = codex_item_type(item);
    if method == "item/started" {
        return CODEX_TOOL_ITEMS
            .contains(&item_type.as_str())
            .then(|| codex_tool(item, &item_type, method, "started"));
    }
    codex_completed(item, &item_type, method)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shell_wrappers_unwrap_both_quote_styles() {
        assert_eq!(unwrap_shell("/bin/zsh -lc 'pytest -q tests'"), "pytest -q tests");
        assert_eq!(unwrap_shell("bash -c \"ls -la\""), "ls -la");
        assert_eq!(unwrap_shell("sh -lc 'a' \"b\""), "sh -lc 'a' \"b\"");
        assert_eq!(unwrap_shell("python -c 'x'"), "python -c 'x'");
    }

    #[test]
    fn internal_commands_are_redacted_inside_prose() {
        let text = "Run:\n```\nlimina _agent finding create \\\n --title x\n```\nDone.";
        let redacted = redact_internal_commands(text);
        assert!(!redacted.contains("limina _agent"));
        assert!(!redacted.contains("--title"));
        assert!(redacted.contains(INTERNAL_COMMAND_LABEL));
        assert!(redacted.ends_with("Done."));
    }

    #[test]
    fn detail_keeps_the_python_key_order() {
        let event = map_codex_notification("error", &json!({"error": {"message": "x"}})).unwrap();
        let keys: Vec<&str> = event.detail.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "method",
                "item_type",
                "kind",
                "text",
                "tool",
                "input",
                "item_id",
                "parent_item_id",
                "phase",
                "state",
                "exit_code",
                "duration_ms",
                "files"
            ]
        );
        let payload = event.payload();
        assert_eq!(payload.keys().next().map(String::as_str), Some("summary"));
    }

    #[test]
    fn mcp_arguments_without_short_strings_are_compact_json() {
        let event = map_codex_notification(
            "item/started",
            &json!({"item": {"type": "mcpToolCall", "id": "m", "server": "db",
                     "tool": "query", "arguments": {"limit": 3, "deep": {"x": 1}}}}),
        )
        .unwrap();
        assert_eq!(event.detail["tool"], "mcp:db/query");
        assert_eq!(event.detail["input"], r#"{"limit":3,"deep":{"x":1}}"#);
        let empty = map_codex_notification(
            "item/started",
            &json!({"item": {"type": "dynamicToolCall", "id": "d", "namespace": "ns",
                     "tool": "t", "arguments": {}}}),
        )
        .unwrap();
        assert_eq!(empty.detail["tool"], "ns/t");
        assert_eq!(empty.detail["input"], Value::Null);
    }
}
