//! Port of `tests/test_cloud_runtime_events.py`: provider streams become the conversation schema.
//!
//! The Python fakes mimicked the SDKs' typed objects; these tests feed the wire JSON those
//! objects were parsed from, which is what the Rust mappers read.

#![allow(clippy::unwrap_used)]

use limina_engines::events::{
    ClaudeToolNames, KNOWLEDGE_SUMMARY, RuntimeEvent, TEXT_LIMIT, is_tool_call, map_claude_message,
    map_codex_notification, redact_internal_commands,
};
use serde_json::{Value, json};

fn long_markdown() -> String {
    format!("## Findings\n\n{}", "The reranker improves NDCG on every slice. ".repeat(60))
}

fn checkpoint_json(summary: &str, status: &str) -> String {
    json!({
        "summary": summary,
        "status": status,
        "current_objective": "Rank",
        "next_step": "Measure",
        "blocker": "None",
    })
    .to_string()
}

fn details(events: &[RuntimeEvent]) -> String {
    serde_json::to_string(&events.iter().map(|event| &event.detail).collect::<Vec<_>>()).unwrap()
}

// Claude stream-json messages, as the CLI writes them.

fn assistant(content: Value) -> Value {
    json!({"type": "assistant", "message": {"id": "msg", "model": "claude-test", "role": "assistant",
           "content": content}, "parent_tool_use_id": null, "session_id": "s"})
}

fn nested_assistant(content: Value, parent: &str) -> Value {
    let mut message = assistant(content);
    message["parent_tool_use_id"] = json!(parent);
    message
}

fn user(content: Value) -> Value {
    json!({"type": "user", "message": {"role": "user", "content": content}, "parent_tool_use_id": null})
}

fn tool_use(id: &str, name: &str, input: Value) -> Value {
    json!({"type": "tool_use", "id": id, "name": name, "input": input})
}

fn tool_result(id: &str, content: Value, is_error: Option<bool>) -> Value {
    json!({"type": "tool_result", "tool_use_id": id, "content": content, "is_error": is_error})
}

fn text_block(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

fn task_started() -> Value {
    json!({"type": "system", "subtype": "task_started", "task_id": "task-1",
           "tool_use_id": "tool-task", "description": "Benchmark the reranker on the frozen set",
           "uuid": "u", "session_id": "s"})
}

fn task_notification() -> Value {
    json!({"type": "system", "subtype": "task_notification", "task_id": "task-1",
           "tool_use_id": "tool-task", "status": "completed",
           "summary": "Benchmark finished: +12% NDCG", "output_file": "", "uuid": "u", "session_id": "s"})
}

fn map_claude(message: &Value) -> Vec<RuntimeEvent> {
    map_claude_message(message, &mut ClaudeToolNames::new())
}

#[test]
fn every_text_and_tool_block_becomes_a_transcript_event() {
    let markdown = long_markdown();
    let events = map_claude(&assistant(json!([
        {"type": "thinking", "thinking": "private reasoning", "signature": "signature"},
        text_block(&markdown),
        tool_use("tool-1", "Bash", json!({"command": "pytest -q\n  tests/unit"})),
        tool_use("tool-2", "Read", json!({"file_path": "src/rank.py", "limit": 40})),
        tool_use("tool-3", "StructuredOutput", json!({"summary": "checkpoint"})),
        text_block(&checkpoint_json("checkpoint", "RUNNING")),
    ])));

    let kinds: Vec<&str> = events.iter().map(|event| event.detail_str("kind").unwrap()).collect();
    assert_eq!(kinds, ["message", "tool", "tool"]);
    let (message, bash, read) = (&events[0], &events[1], &events[2]);
    assert_eq!(message.event_type, "runtime.claude-code");
    assert_eq!(message.detail["text"], markdown.trim());
    assert!(message.detail_str("text").unwrap().len() > 1000);
    assert_eq!(message.summary, "## Findings");
    assert_eq!(message.detail["phase"], "completed");
    assert_eq!(bash.summary, "Using Bash");
    assert_eq!(bash.detail["tool"], "Bash");
    assert_eq!(bash.detail["input"], "pytest -q tests/unit");
    assert_eq!(
        (&bash.detail["item_id"], &bash.detail["phase"], &bash.detail["state"]),
        (&json!("tool-1"), &json!("started"), &json!("running"))
    );
    assert_eq!(read.detail["input"], "src/rank.py");
    assert!(!details(&events).contains("private reasoning"));
    assert!(!details(&events).contains("checkpoint"));
}

#[test]
fn tool_results_complete_their_calls_without_exposing_output() {
    let mut tools = ClaudeToolNames::new();
    map_claude_message(
        &assistant(json!([
            tool_use("tool-1", "Bash", json!({"command": "cat results.csv"})),
            tool_use("tool-2", "WebFetch", json!({"url": "https://example.test/paper"})),
            tool_use("tool-3", "StructuredOutput", json!({"summary": "checkpoint"})),
        ])),
        &mut tools,
    );

    let events = map_claude_message(
        &user(json!([
            tool_result("tool-1", json!("SECRET-RESULT-OUTPUT"), Some(false)),
            tool_result("tool-2", json!([{"type": "text", "text": "SECRET"}]), Some(true)),
            tool_result("tool-3", json!("Structured output accepted"), None),
        ])),
        &mut tools,
    );

    assert_eq!(events.len(), 2);
    let (bash, fetch) = (&events[0], &events[1]);
    assert_eq!(
        (&bash.detail["kind"], &bash.detail["tool"], &bash.detail["item_id"]),
        (&json!("tool"), &json!("Bash"), &json!("tool-1"))
    );
    assert_eq!((&bash.detail["phase"], &bash.detail["state"]), (&json!("completed"), &json!("ok")));
    assert_eq!(bash.summary, "Bash finished");
    assert_eq!(
        (&fetch.detail["tool"], &fetch.detail["state"]),
        (&json!("WebFetch"), &json!("failed"))
    );
    assert!(!details(&events).contains("SECRET"));
}

#[test]
fn internal_knowledge_commands_never_expose_their_command_line() {
    let mut tools = ClaudeToolNames::new();
    let command = "limina _agent finding create --thread private-thread --title x";
    let started = map_claude_message(
        &assistant(json!([tool_use("tool-9", "Bash", json!({"command": command}))])),
        &mut tools,
    )
    .remove(0);
    let completed =
        map_claude_message(&user(json!([tool_result("tool-9", json!("F001"), None)])), &mut tools)
            .remove(0);

    for event in [&started, &completed] {
        assert_eq!(event.summary, KNOWLEDGE_SUMMARY);
        assert_eq!(event.detail["kind"], "knowledge");
        assert_eq!(event.detail["tool"], "limina");
        assert_eq!(event.detail["input"], Value::Null);
        assert!(!serde_json::to_string(&event.detail).unwrap().contains("private-thread"));
    }
    assert!(is_tool_call(&started.detail));
    assert!(!is_tool_call(&completed.detail));

    // The public boundary (`operations.public_event`, ported with the server) is a second line
    // of defense; its prose redaction is this function.
    let text = format!("Recording the finding:\n\n```bash\n{command}\n```\nDone.");
    let redacted = redact_internal_commands(&text);
    assert!(!redacted.contains("private-thread"));
    assert!(!redacted.contains("limina _agent"));
    assert!(redacted.contains("Recording the finding"));
    assert!(redacted.contains("Done."));
}

#[test]
fn tool_inputs_are_summarized_per_tool() {
    let cases = [
        ("Glob", json!({"pattern": "**/*.py", "path": "src"}), "Glob", "**/*.py in src"),
        ("Grep", json!({"pattern": "rerank"}), "Grep", "rerank"),
        ("WebSearch", json!({"query": "listwise reranking"}), "WebSearch", "listwise reranking"),
        (
            "Task",
            json!({"description": "Audit baselines", "prompt": "long"}),
            "Task",
            "Audit baselines",
        ),
        ("NotebookEdit", json!({"notebook_path": "eval.ipynb"}), "NotebookEdit", "eval.ipynb"),
        ("TodoWrite", json!({"todos": [{}, {}, {}]}), "TodoWrite", "3 todos"),
        ("mcp__github__search_issues", json!({"q": "bug"}), "mcp:github/search_issues", "bug"),
        ("Custom", json!({"count": 3, "label": "short value"}), "Custom", "short value"),
    ];
    for (name, values, tool, expected) in cases {
        let event = map_claude(&assistant(json!([tool_use(&format!("id-{name}"), name, values)])))
            .remove(0);
        assert_eq!(event.detail["tool"], tool, "{name}");
        assert_eq!(event.detail["input"], expected, "{name}");
    }
}

#[test]
fn subagent_progress_is_a_status_row_nested_under_its_tool_call() {
    let started = map_claude(&task_started()).remove(0);
    let finished = map_claude(&task_notification()).remove(0);
    let nested =
        map_claude(&nested_assistant(json!([text_block("Subagent note")]), "tool-task")).remove(0);

    assert_eq!(started.detail["kind"], "status");
    assert_eq!(started.detail["text"], "Benchmark the reranker on the frozen set");
    assert_eq!(
        (&started.detail["phase"], &started.detail["state"]),
        (&json!("started"), &json!("running"))
    );
    assert_eq!(finished.detail["text"], "Benchmark finished: +12% NDCG");
    assert_eq!(finished.detail["state"], "ok");
    assert_eq!(started.detail["item_id"], finished.detail["item_id"]);
    assert_eq!(started.detail["parent_item_id"], "tool-task");
    assert_eq!(nested.detail["parent_item_id"], "tool-task");
}

// Codex app-server notifications.

fn codex(method: &str, item: Value) -> Option<RuntimeEvent> {
    map_codex_notification(method, &json!({"threadId": "thread", "turnId": "turn", "item": item}))
}

fn agent_message(text: &str, id: &str) -> Value {
    json!({"type": "agentMessage", "id": id, "text": text})
}

fn plan(text: &str, id: &str) -> Value {
    json!({"type": "plan", "id": id, "text": text})
}

fn command(extra: Value) -> Value {
    let mut item = json!({"type": "commandExecution", "id": "command-1",
                          "command": "/bin/zsh -lc 'pytest -q tests'", "cwd": "/workspace",
                          "commandActions": [], "status": "inProgress", "exitCode": null,
                          "durationMs": null});
    item.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    item
}

fn file_change() -> Value {
    json!({"type": "fileChange", "id": "patch-1", "status": "completed", "changes": [
        {"path": "src/rank.py", "kind": {"type": "update", "move_path": null}, "diff": "-old\n+SECRET_DIFF_LINE"},
        {"path": "docs/notes.md", "kind": {"type": "add"}, "diff": "+SECRET_DIFF_LINE"},
    ]})
}

#[test]
fn agent_messages_and_plans_keep_full_markdown() {
    let markdown = long_markdown();
    let message = codex("item/completed", agent_message(&markdown, "message-1")).unwrap();
    let plan = codex("item/completed", plan("1. Baseline\n2. Treatment", "plan-1")).unwrap();
    let huge =
        codex("item/completed", agent_message(&"x".repeat(TEXT_LIMIT + 50), "message-2")).unwrap();

    assert_eq!(message.event_type, "runtime.codex");
    assert_eq!(message.detail["kind"], "message");
    assert_eq!(message.detail["text"], markdown.trim());
    assert_eq!(message.detail["item_id"], "message-1");
    assert_eq!(message.summary, "## Findings");
    assert_eq!(
        (&plan.detail["kind"], &plan.detail["text"]),
        (&json!("plan"), &json!("1. Baseline\n2. Treatment"))
    );
    assert_eq!(huge.detail_str("text").unwrap().chars().count(), TEXT_LIMIT);
}

#[test]
fn structured_checkpoint_answer_is_not_repeated_as_a_message() {
    let checkpoint = checkpoint_json("Done", "COMPLETE");
    assert_eq!(codex("item/completed", agent_message(&checkpoint, "message-1")), None);
}

#[test]
fn command_rows_merge_by_item_id_and_report_exit_code() {
    let started = codex("item/started", command(json!({}))).unwrap();
    let completed = codex(
        "item/completed",
        command(json!({"status": "failed", "exitCode": 1, "durationMs": 1234,
                       "aggregatedOutput": "SECRET-COMMAND-OUTPUT"})),
    )
    .unwrap();

    assert_eq!(started.summary, "Using command");
    assert_eq!(started.detail["tool"], "command");
    assert_eq!(started.detail["input"], "pytest -q tests");
    assert_eq!(
        (&started.detail["phase"], &started.detail["state"]),
        (&json!("started"), &json!("running"))
    );
    assert_eq!(completed.detail["item_id"], started.detail["item_id"]);
    assert_eq!(
        (&completed.detail["phase"], &completed.detail["state"]),
        (&json!("completed"), &json!("failed"))
    );
    assert_eq!(completed.detail["exit_code"], 1);
    assert_eq!(completed.detail["duration_ms"], 1234);
    assert!(!serde_json::to_string(&completed.detail).unwrap().contains("SECRET"));
    assert!(is_tool_call(&started.detail));
    assert!(!is_tool_call(&completed.detail));
}

#[test]
fn internal_knowledge_commands_are_reported_as_knowledge() {
    let event = codex(
        "item/started",
        command(json!({"command": "bash -lc 'limina _agent experiment claim E001 --thread t-1'"})),
    )
    .unwrap();
    assert_eq!(event.summary, KNOWLEDGE_SUMMARY);
    assert_eq!(
        (&event.detail["kind"], &event.detail["tool"]),
        (&json!("knowledge"), &json!("limina"))
    );
    assert_eq!(event.detail["input"], Value::Null);
    assert!(!serde_json::to_string(&event.detail).unwrap().contains("t-1"));
}

#[test]
fn file_changes_list_paths_and_kinds_but_never_diffs() {
    assert_eq!(codex("item/started", file_change()), None);
    let event = codex("item/completed", file_change()).unwrap();

    assert_eq!(event.summary, "Edited 2 files");
    assert_eq!((&event.detail["kind"], &event.detail["tool"]), (&json!("tool"), &json!("edit")));
    assert_eq!(
        (&event.detail["phase"], &event.detail["state"]),
        (&json!("completed"), &json!("ok"))
    );
    assert_eq!(
        event.detail["files"],
        json!([{"path": "src/rank.py", "kind": "update"}, {"path": "docs/notes.md", "kind": "add"}])
    );
    let serialized = serde_json::to_string(&event.detail).unwrap();
    assert!(!serialized.contains("SECRET_DIFF_LINE"));
    assert!(!serialized.contains("diff"));
    assert!(is_tool_call(&event.detail));
}

#[test]
fn turn_notifications_and_non_tool_items_are_dropped() {
    let dropped = [
        codex("turn/started", Value::Null),
        codex("turn/completed", Value::Null),
        codex("item/agentMessage/delta", Value::Null),
        codex("item/started", agent_message("partial", "message-1")),
        codex("item/started", plan("partial", "plan-1")),
        codex(
            "item/completed",
            json!({"type": "reasoning", "id": "reasoning-1",
                                        "summary": ["private chain of thought"], "content": []}),
        ),
        codex(
            "item/completed",
            json!({"type": "userMessage", "id": "user-1", "content": ["prompt"]}),
        ),
    ];
    assert!(dropped.iter().all(Option::is_none));
}

#[test]
fn errors_become_failed_status_rows() {
    let event = map_codex_notification(
        "error",
        &json!({"error": {"message": "Stream disconnected."}, "willRetry": true,
                "threadId": "thread", "turnId": "turn"}),
    )
    .unwrap();
    assert_eq!(
        (&event.detail["kind"], &event.detail["state"]),
        (&json!("status"), &json!("failed"))
    );
    assert_eq!(event.detail["text"], "Stream disconnected. Codex will retry.");
}

#[test]
fn real_sdk_items_are_understood() {
    let command = json!({
        "type": "commandExecution",
        "id": "cmd-7",
        "command": "bash -lc 'ls -la'",
        "commandActions": [],
        "cwd": "/workspace",
        "status": "completed",
        "exitCode": 0,
        "durationMs": 9,
        "aggregatedOutput": "SECRET",
    });
    let search = json!({"type": "webSearch", "id": "search-1", "query": "listwise reranking"});
    let change = json!({
        "type": "fileChange",
        "id": "patch-2",
        "status": "declined",
        "changes": [{"path": "a.py", "kind": {"type": "delete"}, "diff": "SECRET"}],
    });

    let finished = codex("item/completed", command).unwrap();
    assert_eq!(
        (&finished.detail["state"], &finished.detail["exit_code"]),
        (&json!("ok"), &json!(0))
    );
    assert_eq!(finished.detail["input"], "ls -la");
    let searched = codex("item/started", search).unwrap();
    assert_eq!(
        (&searched.detail["tool"], &searched.detail["input"]),
        (&json!("web_search"), &json!("listwise reranking"))
    );
    let declined = codex("item/completed", change).unwrap();
    assert_eq!(declined.detail["state"], "declined");
    assert_eq!(declined.detail["files"], json!([{"path": "a.py", "kind": "delete"}]));
    assert!(!details(&[finished, declined]).contains("SECRET"));
}

// Beyond the Python suite: the remaining item kinds and the Claude-only message kinds.

#[test]
fn remaining_codex_items_map_to_their_tools() {
    let mcp = codex(
        "item/started",
        json!({"type": "mcpToolCall", "id": "m-1", "server": "github",
        "tool": "search", "arguments": {"q": "rerank"}, "status": "inProgress"}),
    )
    .unwrap();
    assert_eq!(
        (&mcp.detail["tool"], &mcp.detail["input"]),
        (&json!("mcp:github/search"), &json!("rerank"))
    );
    let dynamic = codex(
        "item/completed",
        json!({"type": "dynamicToolCall", "id": "d-1",
        "tool": "lookup", "arguments": "plain", "status": "completed", "success": false}),
    )
    .unwrap();
    assert_eq!(
        (&dynamic.detail["tool"], &dynamic.detail["state"]),
        (&json!("lookup"), &json!("failed"))
    );
    assert_eq!(dynamic.summary, "lookup failed");
    let subagent = codex(
        "item/started",
        json!({"type": "collabAgentToolCall", "id": "c-1",
        "tool": "spawnAgent", "prompt": null, "status": "inProgress"}),
    )
    .unwrap();
    assert_eq!(
        (&subagent.detail["tool"], &subagent.detail["input"]),
        (&json!("subagent"), &json!("spawnAgent"))
    );
    let page = codex(
        "item/started",
        json!({"type": "webSearch", "id": "w-2", "query": "",
        "action": {"type": "openPage", "url": "https://example.test"}}),
    )
    .unwrap();
    assert_eq!(page.detail["input"], "https://example.test");
    let compaction =
        codex("item/completed", json!({"type": "contextCompaction", "id": "k-1"})).unwrap();
    assert_eq!(
        (&compaction.detail["kind"], &compaction.summary.as_str()),
        (&json!("status"), &"Context compacted")
    );
    let one_file = codex(
        "item/completed",
        json!({"type": "fileChange", "id": "p", "status": "failed",
        "changes": [{"path": "a.py", "kind": {"type": "update"}, "diff": ""}]}),
    )
    .unwrap();
    assert_eq!(
        (one_file.summary.as_str(), &one_file.detail["state"]),
        ("Edited a.py", &json!("failed"))
    );
    let declined = codex("item/completed", command(json!({"status": "declined"}))).unwrap();
    assert_eq!(declined.summary, "command was declined");
}

#[test]
fn claude_compaction_server_tools_and_string_content() {
    let compacted =
        map_claude(&json!({"type": "system", "subtype": "compact_boundary", "session_id": "s"}));
    assert_eq!(compacted[0].detail["method"], "system/compact");
    assert_eq!(compacted[0].detail["text"], "Context compacted");
    assert!(
        map_claude(&json!({"type": "system", "subtype": "init", "session_id": "s"})).is_empty()
    );
    assert!(
        map_claude(
            &json!({"type": "user", "message": {"role": "user", "content": "plain prompt"}})
        )
        .is_empty()
    );
    assert!(map_claude(&json!({"type": "result", "subtype": "success"})).is_empty());

    let mut tools = ClaudeToolNames::new();
    let used = map_claude_message(
        &assistant(json!([{"type": "server_tool_use", "id": "srv-1", "name": "web_search",
                            "input": {"query": "rerankers"}}])),
        &mut tools,
    );
    assert_eq!(
        (&used[0].detail["item_type"], &used[0].detail["input"]),
        (&json!("ServerToolUseBlock"), &json!("rerankers"))
    );
    let failed = map_claude_message(
        &assistant(json!([{"type": "advisor_tool_result", "tool_use_id": "srv-1",
                            "content": {"type": "advisor_tool_result_error"}}])),
        &mut tools,
    );
    assert_eq!(
        (&failed[0].detail["tool"], &failed[0].detail["state"]),
        (&json!("web_search"), &json!("failed"))
    );
    let user_text = map_claude_message(&user(json!([text_block("ignored")])), &mut tools);
    assert!(user_text.is_empty(), "user text blocks are prompts, not transcript");
}
