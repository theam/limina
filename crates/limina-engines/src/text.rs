//! Python string semantics the persisted shapes depend on.
//!
//! Event summaries, transcript text and checkpoints were produced by Python (`str.split()`,
//! `str.strip()`, `str.splitlines()`, code-point lengths, `str()` of JSON values). Rows written
//! by either runtime must look the same, so these helpers reproduce those rules exactly.

use serde_json::Value;

/// Python's `str.isspace()` for one character: Unicode white space plus the ASCII separators
/// U+001C–U+001F, which Rust's `char::is_whitespace` does not include.
pub(crate) fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// `str.strip()`.
pub(crate) fn strip(text: &str) -> &str {
    text.trim_matches(is_space)
}

/// `text if len(text) <= limit else text[: limit - 1] + "…"`, counting code points.
pub(crate) fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut clipped: String = text.chars().take(limit.saturating_sub(1)).collect();
    clipped.push('…');
    clipped
}

/// `" ".join(value.split())`, clipped.
pub(crate) fn one_line(value: &str, limit: usize) -> String {
    let joined =
        value.split(is_space).filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" ");
    clip(&joined, limit)
}

/// `str.splitlines()`: every line boundary Python recognizes, `\r\n` counted once.
pub(crate) fn splitlines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        let boundary = matches!(
            c,
            '\n' | '\r'
                | '\u{0b}'
                | '\u{0c}'
                | '\u{1c}'
                | '\u{1d}'
                | '\u{1e}'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if !boundary {
            continue;
        }
        lines.push(&text[start..index]);
        let mut end = index + c.len_utf8();
        if c == '\r'
            && let Some((next_index, '\n')) = chars.peek().copied()
        {
            chars.next();
            end = next_index + 1;
        }
        start = end;
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Python truthiness of a JSON value (`None`, `False`, `0`, `""`, `[]` and `{}` are false).
pub(crate) fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

/// `str(value)` for a JSON value. Containers are rendered as compact JSON, which only differs
/// from Python's `repr` for values no provider sends in these positions.
pub(crate) fn py_str(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// `_string(value)`: a stripped, non-empty string, or `None`.
pub(crate) fn non_empty(value: Option<&Value>) -> Option<String> {
    let text = strip(value?.as_str()?);
    (!text.is_empty()).then(|| text.to_owned())
}

/// `int(value)` as the usage normalizer applied it: integers, floats (truncated), numeric
/// strings and booleans; anything else is `None`.
pub(crate) fn py_int(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(flag) => Some(i64::from(*flag)),
        Value::Number(number) => number.as_i64().or_else(|| {
            let truncated = number.as_f64().filter(|n| n.is_finite())?.trunc();
            // `as` saturates; the range check keeps out-of-range floats as `None`.
            (truncated >= i64::MIN as f64 && truncated <= i64::MAX as f64)
                .then_some(truncated as i64)
        }),
        Value::String(text) => strip(text).replace('_', "").parse().ok(),
        _ => None,
    }
}

/// `_optional_int(value)` from the event mapper: integers only (booleans and floats are not).
pub(crate) fn strict_int(value: Option<&Value>) -> Option<i64> {
    value?.as_i64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn whitespace_and_lines_follow_python() {
        assert_eq!(one_line("  pytest -q\n  tests/unit\u{1f}x ", 500), "pytest -q tests/unit x");
        assert_eq!(splitlines("a\r\nb\rc\u{2028}d\n"), vec!["a", "b", "c", "d"]);
        assert_eq!(splitlines(""), Vec::<&str>::new());
        assert_eq!(splitlines("\n\nx"), vec!["", "", "x"]);
        assert_eq!(strip("\u{1c} x \u{a0}"), "x");
    }

    #[test]
    fn clipping_counts_code_points() {
        assert_eq!(clip("ééé", 3), "ééé");
        assert_eq!(clip("éééé", 3), "éé…");
        assert_eq!(clip(&"x".repeat(10), 4).chars().count(), 4);
    }

    #[test]
    fn json_values_convert_like_python() {
        assert_eq!(py_str(&json!(null)), "None");
        assert_eq!(py_str(&json!(true)), "True");
        assert_eq!(py_str(&json!(3)), "3");
        assert!(!truthy(&json!({})));
        assert!(truthy(&json!({"a": 1})));
        assert_eq!(py_int(&json!(12.9)), Some(12));
        assert_eq!(py_int(&json!(" 7 ")), Some(7));
        assert_eq!(py_int(&json!(true)), Some(1));
        assert_eq!(py_int(&json!("x")), None);
        assert_eq!(strict_int(Some(&json!(true))), None);
        assert_eq!(strict_int(Some(&json!(1.5))), None);
    }
}
