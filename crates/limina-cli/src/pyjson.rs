//! JSON text exactly as Python's `json.dumps(value, ensure_ascii=False)` writes it.
//!
//! Machine-readable output (`--json`, the hidden `_agent` protocol, `watch --json` and JSON error
//! bodies) is parsed by agents and scripts written against the Python CLI, so it keeps Python's
//! separators (`", "` and `": "` compact, `","` and `": "` indented), its float notation
//! (`1e-05`, `1e+16`, `2.0`) and its escaping (non-ASCII characters are written as is).

use std::io;

use serde::Serialize;
use serde_json::Value;
use serde_json::ser::{Formatter, PrettyFormatter};

/// `json.dumps(value, ensure_ascii=False)`.
pub fn dumps(value: &Value) -> String {
    render(value, PythonCompact)
}

/// `json.dumps(value, ensure_ascii=False, indent=2)`.
pub fn dumps_pretty(value: &Value) -> String {
    render(value, PythonPretty(PrettyFormatter::with_indent(b"  ")))
}

fn render<F: Formatter>(value: &Value, formatter: F) -> String {
    let mut buffer = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut buffer, formatter);
    // Serializing a `Value` into memory cannot fail: keys are strings and numbers are finite.
    if value.serialize(&mut serializer).is_err() {
        return value.to_string();
    }
    String::from_utf8(buffer).unwrap_or_else(|_| value.to_string())
}

/// Python's default compact separators: `", "` and `": "`.
struct PythonCompact;

impl Formatter for PythonCompact {
    fn begin_array_value<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        if first { Ok(()) } else { writer.write_all(b", ") }
    }

    fn begin_object_key<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        if first { Ok(()) } else { writer.write_all(b", ") }
    }

    fn begin_object_value<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        writer.write_all(b": ")
    }

    fn write_f64<W: ?Sized + io::Write>(&mut self, writer: &mut W, value: f64) -> io::Result<()> {
        writer.write_all(float_repr(value).as_bytes())
    }

    fn write_f32<W: ?Sized + io::Write>(&mut self, writer: &mut W, value: f32) -> io::Result<()> {
        writer.write_all(float_repr(f64::from(value)).as_bytes())
    }
}

/// `indent=2`: serde_json's pretty layout already matches Python's; only floats differ.
struct PythonPretty<'a>(PrettyFormatter<'a>);

impl Formatter for PythonPretty<'_> {
    fn begin_array<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.begin_array(writer)
    }

    fn end_array<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.end_array(writer)
    }

    fn begin_array_value<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        self.0.begin_array_value(writer, first)
    }

    fn end_array_value<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.end_array_value(writer)
    }

    fn begin_object<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.begin_object(writer)
    }

    fn end_object<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.end_object(writer)
    }

    fn begin_object_key<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        self.0.begin_object_key(writer, first)
    }

    fn begin_object_value<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.begin_object_value(writer)
    }

    fn end_object_value<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.0.end_object_value(writer)
    }

    fn write_f64<W: ?Sized + io::Write>(&mut self, writer: &mut W, value: f64) -> io::Result<()> {
        writer.write_all(float_repr(value).as_bytes())
    }

    fn write_f32<W: ?Sized + io::Write>(&mut self, writer: &mut W, value: f32) -> io::Result<()> {
        writer.write_all(float_repr(f64::from(value)).as_bytes())
    }
}

/// Python's `repr(float)`: the shortest round-trip digits, positional notation when the decimal
/// exponent is in `-4..16` (always with a fractional part), scientific notation otherwise
/// (`1e-05`, `1.5e+16`).
pub fn float_repr(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    // Rust's `{:e}` is the shortest representation that round-trips, like Python's repr.
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((scientific.as_str(), "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let (sign, mantissa) = match mantissa.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", mantissa),
    };
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let mut out = String::from(sign);
    if (-4..16).contains(&exponent) {
        if exponent >= 0 {
            let integer_len = usize::try_from(exponent).unwrap_or(0) + 1;
            if digits.len() <= integer_len {
                out.push_str(&digits);
                out.push_str(&"0".repeat(integer_len - digits.len()));
                out.push_str(".0");
            } else {
                out.push_str(&digits[..integer_len]);
                out.push('.');
                out.push_str(&digits[integer_len..]);
            }
        } else {
            let zeros = usize::try_from(-exponent - 1).unwrap_or(0);
            out.push_str("0.");
            out.push_str(&"0".repeat(zeros));
            out.push_str(&digits);
        }
    } else {
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if exponent < 0 { '-' } else { '+' });
        out.push_str(&format!("{:02}", exponent.unsigned_abs()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compact_output_uses_python_separators() {
        let value = json!({"id": "H001", "tags": [1, 2, {"a": null}], "empty": {}, "list": []});
        assert_eq!(
            dumps(&value),
            r#"{"id": "H001", "tags": [1, 2, {"a": null}], "empty": {}, "list": []}"#
        );
    }

    #[test]
    fn pretty_output_matches_indent_two() {
        let value = json!({"slug": "x", "items": [1, 2.5], "empty": [], "nested": {}});
        assert_eq!(
            dumps_pretty(&value),
            "{\n  \"slug\": \"x\",\n  \"items\": [\n    1,\n    2.5\n  ],\n  \"empty\": [],\n  \"nested\": {}\n}"
        );
    }

    #[test]
    fn strings_keep_non_ascii_and_escape_controls_like_python() {
        let value = json!({"text": "Café — ünïcode \u{2028} \"q\" \\ \n\t\u{1}\u{7f}"});
        assert_eq!(
            dumps(&value),
            "{\"text\": \"Café — ünïcode \u{2028} \\\"q\\\" \\\\ \\n\\t\\u0001\u{7f}\"}"
        );
    }

    #[test]
    fn floats_follow_python_repr() {
        let cases = [
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.0, "1.0"),
            (2.5, "2.5"),
            (100.0, "100.0"),
            (0.1, "0.1"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (0.000_012_5, "1.25e-05"),
            (123_456_789.123, "123456789.123"),
            (1e15, "1000000000000000.0"),
            (1e16, "1e+16"),
            (1.5e16, "1.5e+16"),
            (1e100, "1e+100"),
            (-3.25e-7, "-3.25e-07"),
            (f64::MAX, "1.7976931348623157e+308"),
        ];
        for (value, expected) in cases {
            assert_eq!(float_repr(value), expected, "{value}");
        }
        assert_eq!(dumps(&json!({"cost": 0.00001})), r#"{"cost": 1e-05}"#);
    }
}
