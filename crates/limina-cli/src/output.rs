//! Command output: Python-compatible JSON for machines, plain aligned text for people.
//!
//! The Python CLI rendered human output with Rich panels, tables and colors. The Rust CLI keeps
//! the same content in plain text; only `--json`, the hidden `_agent` protocol and error bodies
//! are a byte-level contract.

use std::io::{self, Write};

use limina_core::LiminaError;
use serde_json::Value;

use crate::pyjson;

/// Where and how a command reports.
#[derive(Debug, Clone, Copy)]
pub struct Output {
    pub as_json: bool,
}

impl Output {
    /// `_emit`: indented JSON with `--json`, otherwise the human rendering.
    pub fn emit(&self, value: &Value, render: impl FnOnce(&Value) -> String) {
        if self.as_json {
            line(&pyjson::dumps_pretty(value));
        } else {
            let text = render(value);
            if !text.is_empty() {
                line(text.trim_end_matches('\n'));
            }
        }
    }

    /// `_invoke`'s error report. Returns the process exit code for the error.
    pub fn error(&self, error: &LiminaError) -> i32 {
        if self.as_json {
            line(&pyjson::dumps(&error.to_body()));
        } else {
            error_line(&format!("{}  {}", error.code(), error.message));
            if let Some(suggestion) = error.details.get("suggestion").filter(|v| truthy(v)) {
                error_line(&format!("Try: {}", display(suggestion)));
            }
        }
        error.exit_code()
    }
}

/// Writes one line to standard output. A closed pipe (`limina … | head`) is not an error.
pub fn line(text: &str) {
    let mut stdout = io::stdout().lock();
    let _ = writeln!(stdout, "{text}");
    let _ = stdout.flush();
}

/// Writes one line to standard error.
pub fn error_line(text: &str) {
    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "{text}");
}

/// Python's truthiness for JSON values.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

/// Python's `str()` of a decoded JSON value, as f-strings rendered it.
pub fn display(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::String(text) => text.clone(),
        Value::Number(number) => match number.as_f64() {
            Some(float) if number.is_f64() => pyjson::float_repr(float),
            _ => number.to_string(),
        },
        other => pyjson::dumps(other),
    }
}

/// `display` of one field of an object (missing fields render as empty text).
pub fn field(value: &Value, key: &str) -> String {
    value.get(key).map(display).unwrap_or_default()
}

/// A titled block of text (Rich's `Panel`).
pub fn panel(title: &str, body: &str) -> String {
    let rule = "─".repeat(title.chars().count().max(3));
    format!("{title}\n{rule}\n{body}\n")
}

/// A titled table with aligned columns (Rich's `Table`).
pub fn table(title: &str, columns: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = columns.iter().map(|c| c.chars().count()).collect();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(cell.chars().count());
            }
        }
    }
    let format_row = |cells: Vec<String>| -> String {
        let last = cells.len().saturating_sub(1);
        let mut text = String::new();
        for (index, cell) in cells.into_iter().enumerate() {
            if index == last {
                text.push_str(&cell);
            } else {
                let width = widths.get(index).copied().unwrap_or(0);
                let padding = width.saturating_sub(cell.chars().count());
                text.push_str(&cell);
                text.push_str(&" ".repeat(padding + 2));
            }
        }
        text.trim_end().to_owned()
    };
    let mut lines = vec![title.to_owned()];
    lines.push(format_row(columns.iter().map(|c| (*c).to_owned()).collect()));
    lines.push(format_row(widths.iter().map(|w| "─".repeat(*w)).collect()));
    for row in rows {
        lines.push(format_row(row.clone()));
    }
    lines.join("\n") + "\n"
}

/// Python's `str.title()`: uppercase the first cased character of every run of cased
/// characters, lowercase the rest.
pub fn title_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous_cased = false;
    for c in text.chars() {
        let cased = c.is_lowercase() || c.is_uppercase();
        if cased {
            if previous_cased {
                out.extend(c.to_lowercase());
            } else {
                out.extend(c.to_uppercase());
            }
        } else {
            out.push(c);
        }
        previous_cased = cased;
    }
    out
}

/// The user-facing product name of a persisted engine identifier.
pub fn engine_label(value: &Value) -> String {
    let id = display(value);
    limina_domain::Engine::parse(&id).map(|engine| engine.label().to_owned()).unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn title_case_matches_python() {
        assert_eq!(title_case("cli test"), "Cli Test");
        assert_eq!(title_case("abc1def x2y"), "Abc1Def X2Y");
        assert_eq!(title_case("LIVE"), "Live");
        assert_eq!(title_case("it's"), "It'S");
    }

    #[test]
    fn display_follows_python_str() {
        assert_eq!(display(&json!(null)), "None");
        assert_eq!(display(&json!(true)), "True");
        assert_eq!(display(&json!(3)), "3");
        assert_eq!(display(&json!(1.0)), "1.0");
        assert_eq!(display(&json!("x")), "x");
        assert!(!truthy(&json!(0)) && !truthy(&json!("")) && truthy(&json!([1])));
    }

    #[test]
    fn tables_align_columns() {
        let text = table(
            "Projects",
            &["ID", "Status"],
            &[vec!["H001".into(), "PROPOSED".into()], vec!["H10".into(), "X".into()]],
        );
        assert_eq!(text, "Projects\nID    Status\n────  ────────\nH001  PROPOSED\nH10   X\n");
    }

    #[test]
    fn engine_labels_hide_marks() {
        assert_eq!(engine_label(&json!("claude-code")), "Claude");
        assert_eq!(engine_label(&json!("codex")), "Codex");
        assert_eq!(engine_label(&json!("other")), "other");
    }
}
