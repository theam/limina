//! A scripted stand-in for `codex app-server`, `codex login --with-access-token` and the
//! `claude` CLI that speaks their real stdio protocols, for the end-to-end tests.
//!
//! The mode comes from the arguments, as Limina launches them. The behavior comes from a
//! scenario file the test writes into the provider's private directory (`$CODEX_HOME/fake-codex.json`
//! or `$CLAUDE_CONFIG_DIR/fake-claude.json`), because Limina starts providers from an empty
//! environment. Everything the fake receives is appended to `fake-codex.log` or
//! `fake-claude.log` in the same directory, one JSON object per line, so tests can assert the
//! exact messages, arguments and environment Limina used.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test support binary")]

use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "app-server") {
        let home = PathBuf::from(std::env::var("CODEX_HOME").expect("CODEX_HOME"));
        Codex::new(home, &args).serve();
    } else if args.iter().any(|arg| arg == "--with-access-token") {
        let home = PathBuf::from(std::env::var("CODEX_HOME").expect("CODEX_HOME"));
        codex_login(&home, &args);
    } else if args.iter().any(|arg| arg == "--input-format") {
        let dir = PathBuf::from(std::env::var("CLAUDE_CONFIG_DIR").expect("CLAUDE_CONFIG_DIR"));
        Claude::new(dir, &args).serve();
    } else {
        eprintln!("limina-fake-provider: unknown invocation {args:?}");
        std::process::exit(64);
    }
}

fn read_json(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}))
}

fn append_log(path: &Path, entry: &Value) {
    let mut file =
        std::fs::OpenOptions::new().create(true).append(true).open(path).expect("open the log");
    writeln!(file, "{entry}").expect("write the log");
}

fn start_entry(args: &[String]) -> Value {
    let env: BTreeMap<String, String> = std::env::vars().collect();
    json!({"start": {
        "args": args,
        "env": env,
        "cwd": std::env::current_dir().map(|dir| dir.to_string_lossy().into_owned()).unwrap_or_default(),
        "pid": std::process::id(),
    }})
}

fn send(value: &Value) {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{value}").expect("write stdout");
    stdout.flush().expect("flush stdout");
}

fn text(value: &Value, key: &str, default: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or(default).to_owned()
}

/// A project resource value echoed to stderr, to prove Limina redacts error reasons.
fn leak() -> String {
    std::env::var("FAKE_LEAK").unwrap_or_default()
}

fn checkpoint(summary: &str) -> Value {
    json!({
        "summary": summary,
        "status": "COMPLETE",
        "current_objective": "Verify the result.",
        "next_step": "Present the finding.",
        "blocker": "None",
    })
}

// Codex ------------------------------------------------------------------------------------------

#[derive(PartialEq)]
enum CodexWait {
    Nothing,
    Steer,
    Interrupt,
    Approval,
}

struct Codex {
    home: PathBuf,
    scenario: Value,
    log: PathBuf,
    thread_id: String,
    turn_id: String,
    wait: CodexWait,
}

impl Codex {
    fn new(home: PathBuf, args: &[String]) -> Self {
        let scenario = read_json(&home.join("fake-codex.json"));
        let log = home.join("fake-codex.log");
        append_log(&log, &start_entry(args));
        let turn_id = text(&scenario, "turn_id", "turn-1");
        Self { home, scenario, log, thread_id: String::new(), turn_id, wait: CodexWait::Nothing }
    }

    fn respond(&self, id: &Value, result: Value) {
        send(&json!({"id": id, "result": result}));
    }

    fn notify(&self, method: &str, params: Value) {
        send(&json!({"method": method, "params": params}));
    }

    fn turn_params(&self, extra: Value) -> Value {
        let mut params = json!({"threadId": self.thread_id, "turnId": self.turn_id});
        if let (Some(params), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
            params.extend(extra.clone());
        }
        params
    }

    fn turn(&self, status: &str, error: Value) -> Value {
        json!({"id": self.turn_id, "items": [], "status": status, "error": error})
    }

    fn account(&self) -> Value {
        let auth = self.home.join("auth.json");
        if auth.exists() { read_json(&auth) } else { Value::Null }
    }

    fn serve(mut self) {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
            append_log(&self.log, &json!({"received": message}));
            let id = message.get("id").cloned().unwrap_or(Value::Null);
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            match message.get("method").and_then(Value::as_str) {
                Some(method) => self.request(method, &id, &params),
                None if id == json!("approval-1") && self.wait == CodexWait::Approval => {
                    self.wait = CodexWait::Nothing;
                    self.finish("Checkpoint complete.");
                }
                None => {}
            }
        }
        append_log(&self.log, &json!({"eof": true}));
    }

    fn request(&mut self, method: &str, id: &Value, params: &Value) {
        match method {
            "initialize" => self.respond(
                id,
                json!({
                    "userAgent": "codex_python_sdk/0.0.0-fake (Fake OS; arm64) fake",
                    "codexHome": self.home,
                    "platformFamily": "unix",
                    "platformOs": "fake",
                }),
            ),
            "initialized" => {}
            "account/read" => {
                self.respond(id, json!({"account": self.account(), "requiresOpenaiAuth": true}));
            }
            "account/login/start" => self.login(id, params),
            "account/login/cancel" => {
                self.respond(id, json!({"status": "canceled"}));
                let login_id = text(params, "loginId", "");
                self.notify(
                    "account/login/completed",
                    json!({"loginId": login_id, "success": false, "error": "cancelled"}),
                );
            }
            "account/logout" => {
                let _ = std::fs::remove_file(self.home.join("auth.json"));
                self.respond(id, json!({}));
            }
            "model/list" => self.respond(id, json!({"data":[
                {"model":"gpt-test", "displayName":"GPT test", "defaultReasoningEffort":"medium", "supportedReasoningEfforts":[{"reasoningEffort":"medium"},{"reasoningEffort":"xhigh"}]},
                {"model":"project-model", "displayName":"Project model", "defaultReasoningEffort":"low", "supportedReasoningEfforts":[{"reasoningEffort":"low"},{"reasoningEffort":"high"}]}
            ], "nextCursor":null})),
            "thread/start" | "thread/resume" => {
                self.thread_id = if method == "thread/resume" {
                    text(params, "threadId", "")
                } else {
                    text(&self.scenario, "thread_id", "thread-new")
                };
                self.respond(
                    id,
                    json!({"thread": {"id": self.thread_id}, "model": params.get("model"),
                           "cwd": params.get("cwd"), "approvalPolicy": "on-request"}),
                );
                self.notify("thread/started", json!({"thread": {"id": self.thread_id}}));
            }
            "turn/start" => self.start_turn(id),
            "turn/steer" => {
                self.respond(id, json!({"turnId": self.turn_id}));
                if self.wait == CodexWait::Steer {
                    self.wait = CodexWait::Nothing;
                    let steering = params
                        .pointer("/input/0/text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    self.finish(&format!("Steering incorporated: {steering}"));
                }
            }
            "turn/interrupt" => {
                self.respond(id, json!({}));
                if self.wait == CodexWait::Interrupt {
                    self.wait = CodexWait::Nothing;
                    let turn = self.turn("interrupted", Value::Null);
                    self.notify(
                        "turn/completed",
                        json!({"threadId": self.thread_id, "turn": turn}),
                    );
                }
            }
            _ => send(
                &json!({"id": id, "error": {"code": -32601, "message": format!("unknown method {method}")}}),
            ),
        }
    }

    fn login(&mut self, id: &Value, params: &Value) {
        match params.get("type").and_then(Value::as_str) {
            Some("apiKey") => {
                std::fs::write(self.home.join("auth.json"), json!({"type": "apiKey"}).to_string())
                    .unwrap();
                self.respond(id, json!({"type": "apiKey"}));
            }
            Some("chatgptDeviceCode") => {
                self.respond(
                    id,
                    json!({"type": "chatgptDeviceCode", "loginId": "login-1",
                           "verificationUrl": "https://example.invalid/device", "userCode": "WXYZ-1234"}),
                );
                match text(&self.scenario, "login", "success").as_str() {
                    "success" => {
                        let account = json!({"type": "chatgpt", "email": "olivia@example.test", "planType": "pro"});
                        std::fs::write(self.home.join("auth.json"), account.to_string()).unwrap();
                        self.notify(
                            "account/login/completed",
                            json!({"loginId": "login-1", "success": true, "error": null}),
                        );
                    }
                    "fail" => self.notify(
                        "account/login/completed",
                        json!({"loginId": "login-1", "success": false, "error": "expired"}),
                    ),
                    _ => {} // "wait": until account/login/cancel
                }
            }
            _ => {
                send(&json!({"id": id, "error": {"code": -32602, "message": "unsupported login"}}))
            }
        }
    }

    fn start_turn(&mut self, id: &Value) {
        let mode = text(&self.scenario, "turn", "complete");
        if mode == "overloaded" {
            send(
                &json!({"id": id, "error": {"code": -32001, "message": "Server overloaded; retry later.",
                                             "data": {"codex_error_info": "server_overloaded"}}}),
            );
            return;
        }
        if mode == "crash" {
            eprintln!("fatal: the fake provider crashed on purpose {}", leak());
            std::process::exit(3);
        }
        self.respond(id, json!({"turn": self.turn("inProgress", Value::Null)}));
        self.notify(
            "turn/started",
            json!({"threadId": self.thread_id, "turn": self.turn("inProgress", Value::Null)}),
        );
        let command = json!({"type": "commandExecution", "id": "cmd-1", "command": "/bin/zsh -lc 'pytest -q tests'",
                             "cwd": "/workspace", "commandActions": [], "status": "inProgress",
                             "exitCode": null, "durationMs": null});
        self.notify("item/started", self.turn_params(json!({"item": command, "startedAtMs": 1})));
        let mut finished = command.clone();
        finished["status"] = json!("completed");
        finished["exitCode"] = json!(0);
        finished["durationMs"] = json!(12);
        finished["aggregatedOutput"] = json!("SECRET-COMMAND-OUTPUT");
        self.notify(
            "item/completed",
            self.turn_params(json!({"item": finished, "completedAtMs": 2})),
        );
        // Noise the session must ignore: another turn's item and a malformed line.
        self.notify(
            "item/completed",
            json!({"threadId": self.thread_id, "turnId": "other-turn",
            "item": {"type": "agentMessage", "id": "noise", "text": "Not this turn."}}),
        );
        send(&json!("not a JSON-RPC object"));
        println!("this line is not JSON");
        let message = json!({"type": "agentMessage", "id": "msg-1", "phase": "commentary",
                             "text": "## Findings\n\nThe reranker helps."});
        self.notify(
            "item/completed",
            self.turn_params(json!({"item": message, "completedAtMs": 3})),
        );
        match mode.as_str() {
            "steer" => self.wait = CodexWait::Steer,
            "interrupt" => self.wait = CodexWait::Interrupt,
            "approval" => {
                self.wait = CodexWait::Approval;
                send(
                    &json!({"id": "approval-1", "method": "item/commandExecution/requestApproval",
                             "params": self.turn_params(json!({"itemId": "cmd-2", "command": "rm -rf build"}))}),
                );
            }
            "fail" => {
                self.notify(
                    "error",
                    self.turn_params(
                        json!({"error": {"message": "Stream disconnected."}, "willRetry": false}),
                    ),
                );
                let turn = self.turn("failed", json!({"message": "Upstream model failure"}));
                self.notify("turn/completed", json!({"threadId": self.thread_id, "turn": turn}));
            }
            "no-decision" => {
                self.notify("turn/completed", json!({"threadId": self.thread_id, "turn": self.turn("completed", Value::Null)}));
            }
            "hang" => {}
            _ => self.finish("Checkpoint complete."),
        }
    }

    fn finish(&self, summary: &str) {
        let breakdown = |n: i64| {
            json!({"inputTokens": n, "outputTokens": n / 5, "cachedInputTokens": n * 2 / 5,
                                        "reasoningOutputTokens": n / 12, "totalTokens": n + n / 5})
        };
        self.notify(
            "thread/tokenUsage/updated",
            self.turn_params(
                json!({"tokenUsage": {"last": breakdown(100), "total": breakdown(9999)}}),
            ),
        );
        let answer = json!({"type": "agentMessage", "id": "msg-final", "phase": "final_answer",
                            "text": checkpoint(summary).to_string()});
        self.notify(
            "item/completed",
            self.turn_params(json!({"item": answer, "completedAtMs": 4})),
        );
        self.notify(
            "turn/completed",
            json!({"threadId": self.thread_id, "turn": self.turn("completed", Value::Null)}),
        );
    }
}

fn codex_login(home: &Path, args: &[String]) {
    let log = home.join("fake-codex.log");
    append_log(&log, &start_entry(args));
    let mut token = String::new();
    std::io::stdin().read_to_string(&mut token).unwrap();
    append_log(&log, &json!({"access_token": token}));
    if token == "valid-access-token" {
        let account = json!({"type": "chatgpt", "email": null, "planType": "team"});
        std::fs::write(home.join("auth.json"), account.to_string()).unwrap();
        println!("Logged in.");
    } else {
        eprintln!("The access token was rejected.");
        std::process::exit(1);
    }
}

// Claude -----------------------------------------------------------------------------------------

enum Step {
    Emit(Value),
    /// Send a `can_use_tool` request and wait for its answer.
    Ask(String, Value),
    /// Wait for an interrupt, then emit this.
    AwaitInterrupt(Value),
    Exit(i32),
}

struct Claude {
    scenario: Value,
    log: PathBuf,
    session_id: String,
    users: usize,
    script: VecDeque<Step>,
    pending: Option<String>,
    interrupt_reply: Option<Value>,
    requests: usize,
}

impl Claude {
    fn new(dir: PathBuf, args: &[String]) -> Self {
        let scenario = read_json(&dir.join("fake-claude.json"));
        let log = dir.join("fake-claude.log");
        append_log(&log, &start_entry(args));
        let session_id = text(&scenario, "session_id", "claude-session");
        Self {
            scenario,
            log,
            session_id,
            users: 0,
            script: VecDeque::new(),
            pending: None,
            interrupt_reply: None,
            requests: 0,
        }
    }

    fn serve(mut self) {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
            append_log(&self.log, &json!({"received": message}));
            match message.get("type").and_then(Value::as_str) {
                Some("control_request") => self.control_request(&message),
                Some("control_response") => {
                    let request_id =
                        message.pointer("/response/request_id").and_then(Value::as_str);
                    if request_id.is_some() && request_id == self.pending.as_deref() {
                        self.pending = None;
                        self.run();
                    }
                }
                Some("user") => {
                    self.users += 1;
                    self.plan_user_message();
                    self.run();
                }
                _ => {}
            }
        }
        append_log(&self.log, &json!({"eof": true}));
    }

    fn control_request(&mut self, message: &Value) {
        let request_id = message.get("request_id").cloned().unwrap_or(Value::Null);
        let subtype = message.pointer("/request/subtype").and_then(Value::as_str).unwrap_or("");
        let body = match subtype {
            "initialize" => {
                json!({"commands": [], "output_style": "default", "models": self.scenario.get("models").cloned().unwrap_or_else(|| json!([
                    {"value":"default", "displayName":"Default", "resolvedModel":"claude-test", "supportsEffort":true, "supportedEffortLevels":["low","high"]},
                    {"value":"fast", "displayName":"Fast"}
                ]))})
            }
            _ => json!({}),
        };
        send(&json!({"type": "control_response",
                     "response": {"subtype": "success", "request_id": request_id, "response": body}}));
        if subtype == "interrupt"
            && let Some(reply) = self.interrupt_reply.take()
        {
            send(&reply);
            self.run();
        }
    }

    fn run(&mut self) {
        while self.pending.is_none() && self.interrupt_reply.is_none() {
            let Some(step) = self.script.pop_front() else { return };
            match step {
                Step::Emit(message) => send(&message),
                Step::Ask(tool_name, input) => {
                    self.requests += 1;
                    let request_id = format!("perm-{}", self.requests);
                    send(&json!({"type": "control_request", "request_id": request_id,
                                 "request": {"subtype": "can_use_tool", "tool_name": tool_name, "input": input,
                                             "permission_suggestions": [], "tool_use_id": "toolu_1"}}));
                    self.pending = Some(request_id);
                }
                Step::AwaitInterrupt(reply) => self.interrupt_reply = Some(reply),
                Step::Exit(code) => {
                    eprintln!("fatal: the fake CLI exited on purpose {}", leak());
                    std::process::exit(code);
                }
            }
        }
    }

    fn system(&self, subtype: &str, extra: Value) -> Value {
        let mut message = json!({"type": "system", "subtype": subtype, "uuid": "u-1", "session_id": self.session_id});
        if let (Some(message), Some(extra)) = (message.as_object_mut(), extra.as_object()) {
            message.extend(extra.clone());
        }
        message
    }

    fn assistant(&self, id: &str, content: Value) -> Value {
        json!({"type": "assistant", "message": {"id": id, "model": "claude-test", "role": "assistant",
               "content": content}, "parent_tool_use_id": null, "session_id": self.session_id, "uuid": "u-2"})
    }

    fn result(&self, is_error: bool, structured: Value) -> Value {
        json!({"type": "result", "subtype": if is_error { "error_during_execution" } else { "success" },
               "duration_ms": 10, "duration_api_ms": 8, "is_error": is_error, "num_turns": 2,
               "session_id": self.session_id, "stop_reason": if is_error { "interrupted" } else { "end_turn" },
               "result": "", "structured_output": structured, "total_cost_usd": 0.0123455,
               "usage": {"input_tokens": 1200, "cache_read_input_tokens": 300, "output_tokens": 80}})
    }

    fn plan_user_message(&mut self) {
        let mode = text(&self.scenario, "scenario", "complete");
        let init = self.system(
            "init",
            json!({"cwd": std::env::current_dir().unwrap(), "model": "claude-test"}),
        );
        let started = self.assistant(
            "message-start",
            json!([{"type": "text", "text": "Starting the baseline."}]),
        );
        let steps = match (mode.as_str(), self.users) {
            ("complete", _) => self.complete_steps(init),
            ("steer", 1) | ("interrupt", _) => vec![
                Step::Emit(init),
                Step::Emit(started),
                Step::AwaitInterrupt(self.result(true, Value::Null)),
            ],
            ("steer", _) => {
                vec![Step::Emit(self.result(false, checkpoint("Steering incorporated.")))]
            }
            ("error-result", _) => vec![
                Step::Emit(init),
                Step::Emit({
                    let mut result = self.result(true, Value::Null);
                    result["stop_reason"] = json!("max_tokens");
                    result
                }),
            ],
            ("exit", _) => vec![Step::Emit(init), Step::Exit(2)],
            _ => vec![Step::Emit(init), Step::Emit(started)], // "hang"
        };
        self.script.extend(steps);
    }

    fn complete_steps(&self, init: Value) -> Vec<Step> {
        let mut steps = vec![
            Step::Emit(init),
            Step::Emit(self.assistant("message-123", json!([
                {"type": "thinking", "thinking": "private reasoning", "signature": "sig"},
                {"type": "text", "text": "Planning the benchmark run."},
                {"type": "tool_use", "id": "toolu_bash", "name": "Bash", "input": {"command": "pytest -q"}},
            ]))),
        ];
        for check in
            self.scenario.get("permissions").and_then(Value::as_array).into_iter().flatten()
        {
            steps.push(Step::Ask(
                text(check, "tool_name", "Read"),
                check.get("input").cloned().unwrap_or(json!({})),
            ));
        }
        steps.extend([
            Step::Emit(json!({"type": "user", "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_bash", "content": "SECRET-OUTPUT", "is_error": false}]},
                "parent_tool_use_id": null, "session_id": self.session_id})),
            Step::Emit(self.system("task_started", json!({"task_id": "task-1", "tool_use_id": "toolu_task",
                "description": "Benchmark the reranker"}))),
            Step::Emit(self.system("task_notification", json!({"task_id": "task-1", "tool_use_id": "toolu_task",
                "status": "completed", "summary": "Benchmark finished: +12% NDCG", "output_file": ""}))),
            Step::Emit(self.system("compact_boundary", json!({}))),
            Step::Emit(self.assistant("message-123", json!([
                {"type": "tool_use", "id": "toolu_out", "name": "StructuredOutput", "input": checkpoint("Claude checkpointed the work.")},
            ]))),
            Step::Emit(self.result(false, checkpoint("Claude checkpointed the work."))),
        ]);
        steps
    }
}
