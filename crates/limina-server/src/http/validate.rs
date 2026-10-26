//! Request validation with FastAPI's error contract.
//!
//! The Python API validated requests with pydantic and answered `422 invalid_request` with the
//! pydantic errors in `details.errors`. Clients and tests match on those entries, so this module
//! reproduces them for the field types the API uses: the same `type`, `loc`, `msg`, `input` and
//! `ctx`, the same lax coercions (`"2"` and `2.0` are integers, `true` is `1`), and errors in
//! declaration order, unknown fields last.
//!
//! ```text
//! {"type": "string_too_short", "loc": ["body", "slug"], "msg": "String should have at least 1 character",
//!  "input": "", "ctx": {"min_length": 1}}
//! ```

use serde_json::{Map, Number, Value, json};

/// The collected errors of one request.
#[derive(Debug, Default)]
pub struct Errors(Vec<Value>);

impl Errors {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn into_vec(self) -> Vec<Value> {
        self.0
    }

    /// Records one pydantic error.
    pub fn push(&mut self, kind: &str, loc: &Loc, msg: impl Into<String>, input: Value) {
        self.0.push(json!({"type": kind, "loc": loc.0, "msg": msg.into(), "input": input}));
    }

    /// Records one pydantic error with its `ctx`.
    pub fn push_ctx(
        &mut self,
        kind: &str,
        loc: &Loc,
        msg: impl Into<String>,
        input: Value,
        ctx: Value,
    ) {
        self.0.push(
            json!({"type": kind, "loc": loc.0, "msg": msg.into(), "input": input, "ctx": ctx}),
        );
    }

    pub fn missing(&mut self, loc: &Loc, input: Value) {
        self.push("missing", loc, "Field required", input);
    }
}

/// Where a value was read from: `["body", "slug"]`, `["query", "limit"]`,
/// `["header", "idempotency-key"]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Loc(Vec<Value>);

impl Loc {
    pub fn body() -> Self {
        Self(vec![json!("body")])
    }

    pub fn query(name: &str) -> Self {
        Self(vec![json!("query"), json!(name)])
    }

    pub fn header(name: &str) -> Self {
        Self(vec![json!("header"), json!(name)])
    }

    /// A field below this location.
    #[must_use]
    pub fn field(&self, name: &str) -> Self {
        let mut parts = self.0.clone();
        parts.push(json!(name));
        Self(parts)
    }

    /// A position below this location (`["body", 12]` for a JSON syntax error).
    #[must_use]
    pub fn index(&self, position: usize) -> Self {
        let mut parts = self.0.clone();
        parts.push(json!(position));
        Self(parts)
    }

    /// The location as the JSON array it is rendered as.
    pub fn to_value(&self) -> Value {
        Value::Array(self.0.clone())
    }
}

// ─── Constraints ─────────────────────────────────────────────────────────────

/// Length limits of a text field, in characters (Python `len`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Text {
    pub min: Option<usize>,
    pub max: Option<usize>,
}

impl Text {
    pub const ANY: Self = Self { min: None, max: None };

    pub const fn min(min: usize) -> Self {
        Self { min: Some(min), max: None }
    }

    pub const fn max(max: usize) -> Self {
        Self { min: None, max: Some(max) }
    }

    pub const fn between(min: usize, max: usize) -> Self {
        Self { min: Some(min), max: Some(max) }
    }
}

/// Bounds of an integer field (`ge`, `le`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Int {
    pub ge: Option<i64>,
    pub le: Option<i64>,
}

impl Int {
    pub const ANY: Self = Self { ge: None, le: None };

    pub const fn ge(ge: i64) -> Self {
        Self { ge: Some(ge), le: None }
    }

    pub const fn between(ge: i64, le: i64) -> Self {
        Self { ge: Some(ge), le: Some(le) }
    }
}

fn plural(count: usize, word: &str) -> String {
    if count == 1 { word.to_owned() } else { format!("{word}s") }
}

/// Checks a string's length (`string_too_short`, `string_too_long`).
pub fn check_text(errors: &mut Errors, loc: &Loc, value: &str, rule: Text) -> bool {
    let length = value.chars().count();
    if let Some(min) = rule.min
        && length < min
    {
        errors.push_ctx(
            "string_too_short",
            loc,
            format!("String should have at least {min} {}", plural(min, "character")),
            json!(value),
            json!({"min_length": min}),
        );
        return false;
    }
    if let Some(max) = rule.max
        && length > max
    {
        errors.push_ctx(
            "string_too_long",
            loc,
            format!("String should have at most {max} {}", plural(max, "character")),
            json!(value),
            json!({"max_length": max}),
        );
        return false;
    }
    true
}

/// Checks a `SecretStr`'s length, which pydantic reports as `too_short` / `too_long` values.
pub fn check_secret(errors: &mut Errors, loc: &Loc, value: &str, rule: Text) -> bool {
    let length = value.chars().count();
    if let Some(min) = rule.min
        && length < min
    {
        errors.push_ctx(
            "too_short",
            loc,
            format!(
                "Value should have at least {min} {} after validation, not {length}",
                plural(min, "item")
            ),
            json!(value),
            json!({"field_type": "Value", "min_length": min, "actual_length": length}),
        );
        return false;
    }
    if let Some(max) = rule.max
        && length > max
    {
        errors.push_ctx(
            "too_long",
            loc,
            format!(
                "Value should have at most {max} {} after validation, not {length}",
                plural(max, "item")
            ),
            json!(value),
            json!({"field_type": "Value", "max_length": max, "actual_length": length}),
        );
        return false;
    }
    true
}

/// Checks an integer against its bounds (`greater_than_equal`, `less_than_equal`).
pub fn check_int(errors: &mut Errors, loc: &Loc, value: i64, rule: Int, input: Value) -> bool {
    if let Some(ge) = rule.ge
        && value < ge
    {
        errors.push_ctx(
            "greater_than_equal",
            loc,
            format!("Input should be greater than or equal to {ge}"),
            input,
            json!({"ge": ge}),
        );
        return false;
    }
    if let Some(le) = rule.le
        && value > le
    {
        errors.push_ctx(
            "less_than_equal",
            loc,
            format!("Input should be less than or equal to {le}"),
            input,
            json!({"le": le}),
        );
        return false;
    }
    true
}

/// `'a'`, `'a' or 'b'`, `'a', 'b' or 'c'`: how pydantic lists a literal's choices.
pub fn expected_literals(allowed: &[&str]) -> String {
    let quoted: Vec<String> = allowed.iter().map(|value| format!("'{value}'")).collect();
    match quoted.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
    }
}

/// Checks a literal choice (`literal_error`); matching is exact, as in pydantic.
pub fn check_literal(errors: &mut Errors, loc: &Loc, input: &Value, allowed: &[&str]) -> bool {
    if input.as_str().is_some_and(|value| allowed.contains(&value)) {
        return true;
    }
    let expected = expected_literals(allowed);
    errors.push_ctx(
        "literal_error",
        loc,
        format!("Input should be {expected}"),
        input.clone(),
        json!({"expected": expected}),
    );
    false
}

// ─── Lax coercions ───────────────────────────────────────────────────────────

/// An integer parsed from a string the way pydantic does: surrounding whitespace, an optional
/// sign, digits with single underscores between them, and an optional all-zero fraction.
/// `Err(true)` when the digits do not fit in an `i64`.
pub fn parse_int(text: &str) -> Result<i64, bool> {
    let trimmed = text.trim();
    let (negative, unsigned) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let (whole, fraction) = match unsigned.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (unsigned, None),
    };
    if let Some(fraction) = fraction
        && (fraction.is_empty() || !fraction.bytes().all(|byte| byte == b'0'))
    {
        return Err(false);
    }
    let valid_digits = !whole.is_empty()
        && !whole.starts_with('_')
        && !whole.ends_with('_')
        && !whole.contains("__")
        && whole.bytes().all(|byte| byte.is_ascii_digit() || byte == b'_');
    if !valid_digits {
        return Err(false);
    }
    let digits: String = whole.chars().filter(|character| *character != '_').collect();
    let magnitude: i128 = digits.parse().map_err(|_| true)?;
    let value = if negative { -magnitude } else { magnitude };
    i64::try_from(value).map_err(|_| true)
}

/// Reads an integer from a JSON value in pydantic's lax mode, or records why not.
pub fn json_int(errors: &mut Errors, loc: &Loc, input: &Value, rule: Int) -> Option<i64> {
    let value = match input {
        Value::Bool(flag) => Some(i64::from(*flag)),
        Value::Number(number) => number_int(errors, loc, number, input),
        Value::String(text) => string_int(errors, loc, text, input),
        _ => {
            errors.push("int_type", loc, "Input should be a valid integer", input.clone());
            None
        }
    }?;
    check_int(errors, loc, value, rule, input.clone()).then_some(value)
}

fn number_int(errors: &mut Errors, loc: &Loc, number: &Number, input: &Value) -> Option<i64> {
    if let Some(value) = number.as_i64() {
        return Some(value);
    }
    let float = number.as_f64().unwrap_or(f64::NAN);
    if float.fract() != 0.0 {
        errors.push(
            "int_from_float",
            loc,
            "Input should be a valid integer, got a number with a fractional part",
            input.clone(),
        );
        return None;
    }
    // Whole floats inside the i64 range convert exactly; anything larger is out of range.
    #[allow(clippy::cast_possible_truncation, reason = "checked to be whole and in range")]
    if float.abs() < 9.0e18 {
        return Some(float as i64);
    }
    errors.push(
        "int_parsing_size",
        loc,
        "Unable to parse input string as an integer, exceeded maximum size",
        input.clone(),
    );
    None
}

fn string_int(errors: &mut Errors, loc: &Loc, text: &str, input: &Value) -> Option<i64> {
    match parse_int(text) {
        Ok(value) => Some(value),
        Err(true) => {
            errors.push(
                "int_parsing_size",
                loc,
                "Unable to parse input string as an integer, exceeded maximum size",
                input.clone(),
            );
            None
        }
        Err(false) => {
            errors.push(
                "int_parsing",
                loc,
                "Input should be a valid integer, unable to parse string as an integer",
                input.clone(),
            );
            None
        }
    }
}

/// Reads an integer from a query string value.
pub fn text_int(errors: &mut Errors, loc: &Loc, text: &str, rule: Int) -> Option<i64> {
    let input = json!(text);
    let value = string_int(errors, loc, text, &input)?;
    check_int(errors, loc, value, rule, input).then_some(value)
}

/// pydantic's string-to-boolean table (case-insensitive, no trimming).
pub fn parse_bool(text: &str) -> Option<bool> {
    match text.to_lowercase().as_str() {
        "1" | "on" | "t" | "true" | "y" | "yes" => Some(true),
        "0" | "off" | "f" | "false" | "n" | "no" => Some(false),
        _ => None,
    }
}

// ─── JSON bodies ─────────────────────────────────────────────────────────────

/// What a JSON route received as its body.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    /// No body, an empty one or JSON `null`.
    Missing,
    /// A body that was not sent as JSON, kept as text for the error's `input`.
    Text(String),
    Json(Value),
}

/// The fields of a request model, read in declaration order. Each reader records its own errors
/// and returns `None` when the field is invalid; [`Fields::finish`] reports undeclared fields as
/// `extra_forbidden` (the models forbid extra fields).
pub struct Fields<'a> {
    object: &'a Map<String, Value>,
    whole: &'a Value,
    loc: Loc,
    errors: &'a mut Errors,
    declared: Vec<&'static str>,
}

impl<'a> Fields<'a> {
    /// Opens a model body: `missing` without one, `model_attributes_type` unless it is an object.
    pub fn open(body: &'a Body, errors: &'a mut Errors) -> Option<Self> {
        let loc = Loc::body();
        match body {
            Body::Missing => {
                errors.missing(&loc, Value::Null);
                None
            }
            Body::Text(text) => {
                model_type_error(errors, &loc, json!(text));
                None
            }
            Body::Json(value @ Value::Object(object)) => {
                Some(Self { object, whole: value, loc, errors, declared: Vec::new() })
            }
            Body::Json(other) => {
                model_type_error(errors, &loc, other.clone());
                None
            }
        }
    }

    pub fn errors(&mut self) -> &mut Errors {
        self.errors
    }

    fn take(&mut self, name: &'static str) -> (Loc, Option<&'a Value>) {
        self.declared.push(name);
        (self.loc.field(name), self.object.get(name))
    }

    fn missing(&mut self, loc: &Loc) {
        self.errors.missing(loc, self.whole.clone());
    }

    /// A required text field.
    pub fn text(&mut self, name: &'static str, rule: Text) -> Option<String> {
        let (loc, value) = self.take(name);
        let Some(value) = value else {
            self.missing(&loc);
            return None;
        };
        string(self.errors, &loc, value, rule)
    }

    /// A text field with a default.
    pub fn text_or(&mut self, name: &'static str, default: &str, rule: Text) -> Option<String> {
        let (loc, value) = self.take(name);
        match value {
            None => Some(default.to_owned()),
            Some(value) => string(self.errors, &loc, value, rule),
        }
    }

    /// A nullable text field (`str | None`); absent and `null` are both `None`.
    pub fn nullable_text(&mut self, name: &'static str, rule: Text) -> Option<Option<String>> {
        let (loc, value) = self.take(name);
        match value {
            None | Some(Value::Null) => Some(None),
            Some(value) => string(self.errors, &loc, value, rule).map(Some),
        }
    }

    /// A required `SecretStr` field.
    pub fn secret(&mut self, name: &'static str, rule: Text) -> Option<String> {
        let (loc, value) = self.take(name);
        let Some(value) = value else {
            self.missing(&loc);
            return None;
        };
        let text = string(self.errors, &loc, value, Text::ANY)?;
        check_secret(self.errors, &loc, &text, rule).then_some(text)
    }

    /// A literal choice, required when `default` is `None`.
    pub fn literal(
        &mut self,
        name: &'static str,
        allowed: &[&str],
        default: Option<&str>,
    ) -> Option<String> {
        let (loc, value) = self.take(name);
        match (value, default) {
            (None, Some(default)) => Some(default.to_owned()),
            (None, None) => {
                self.missing(&loc);
                None
            }
            (Some(value), _) => {
                check_literal(self.errors, &loc, value, allowed).then(|| py_text(value))
            }
        }
    }

    /// A nullable literal choice (`Literal[…] | None`).
    pub fn nullable_literal(
        &mut self,
        name: &'static str,
        allowed: &[&str],
    ) -> Option<Option<String>> {
        let (loc, value) = self.take(name);
        match value {
            None | Some(Value::Null) => Some(None),
            Some(value) => {
                check_literal(self.errors, &loc, value, allowed).then(|| Some(py_text(value)))
            }
        }
    }

    /// An integer, required when `default` is `None`.
    pub fn int(&mut self, name: &'static str, default: Option<i64>, rule: Int) -> Option<i64> {
        let (loc, value) = self.take(name);
        match (value, default) {
            (None, Some(default)) => Some(default),
            (None, None) => {
                self.missing(&loc);
                None
            }
            (Some(value), _) => json_int(self.errors, &loc, value, rule),
        }
    }

    /// A JSON object (`dict[str, Any]`), required unless it defaults to `{}`.
    pub fn object(&mut self, name: &'static str, required: bool) -> Option<Map<String, Value>> {
        let (loc, value) = self.take(name);
        match value {
            None if required => {
                self.missing(&loc);
                None
            }
            None => Some(Map::new()),
            Some(Value::Object(object)) => Some(object.clone()),
            Some(other) => {
                self.errors.push(
                    "dict_type",
                    &loc,
                    "Input should be a valid dictionary",
                    other.clone(),
                );
                None
            }
        }
    }

    /// Reports undeclared fields.
    pub fn finish(self) {
        for (name, value) in self.object {
            if !self.declared.contains(&name.as_str()) {
                self.errors.push(
                    "extra_forbidden",
                    &self.loc.field(name),
                    "Extra inputs are not permitted",
                    value.clone(),
                );
            }
        }
    }

    /// Whether the body set this field at all (pydantic's `exclude_unset`).
    pub fn has(&self, name: &str) -> bool {
        self.object.contains_key(name)
    }
}

fn model_type_error(errors: &mut Errors, loc: &Loc, input: Value) {
    errors.push(
        "model_attributes_type",
        loc,
        "Input should be a valid dictionary or object to extract fields from",
        input,
    );
}

/// A string field (`string_type` for anything else, then the length rule).
pub fn string(errors: &mut Errors, loc: &Loc, value: &Value, rule: Text) -> Option<String> {
    let Value::String(text) = value else {
        errors.push("string_type", loc, "Input should be a valid string", value.clone());
        return None;
    };
    check_text(errors, loc, text, rule).then(|| text.clone())
}

fn py_text(value: &Value) -> String {
    value.as_str().map(str::to_owned).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_parse_like_pydantic() {
        for (text, expected) in [
            ("1_000", Ok(1000)),
            ("2.", Err(false)),
            ("2.00", Ok(2)),
            (" 3 ", Ok(3)),
            ("+4", Ok(4)),
            ("-5", Ok(-5)),
            ("1e3", Err(false)),
            ("0x10", Err(false)),
            ("007", Ok(7)),
            ("1__0", Err(false)),
            ("_1", Err(false)),
            ("", Err(false)),
            ("-0", Ok(0)),
            ("1.0000000001", Err(false)),
            ("\t7\n", Ok(7)),
            ("99999999999999999999999", Err(true)),
        ] {
            assert_eq!(parse_int(text), expected, "{text:?}");
        }
    }

    #[test]
    fn booleans_parse_like_pydantic() {
        assert_eq!(parse_bool("TRUE"), Some(true));
        assert_eq!(parse_bool("off"), Some(false));
        assert_eq!(parse_bool(" true"), None);
        assert_eq!(parse_bool(""), None);
    }

    #[test]
    fn json_integers_follow_lax_mode() {
        let mut errors = Errors::new();
        let loc = Loc::body().field("n");
        assert_eq!(json_int(&mut errors, &loc, &json!(true), Int::ANY), Some(1));
        assert_eq!(json_int(&mut errors, &loc, &json!(2.0), Int::ANY), Some(2));
        assert_eq!(json_int(&mut errors, &loc, &json!("3"), Int::ANY), Some(3));
        assert!(errors.is_empty());
        assert_eq!(json_int(&mut errors, &loc, &json!(1.5), Int::ANY), None);
        assert_eq!(json_int(&mut errors, &loc, &Value::Null, Int::ANY), None);
        assert_eq!(json_int(&mut errors, &loc, &json!(0), Int::ge(1)), None);
        let kinds: Vec<Value> =
            errors.into_vec().iter().map(|error| error["type"].clone()).collect();
        assert_eq!(
            kinds,
            [json!("int_from_float"), json!("int_type"), json!("greater_than_equal")]
        );
    }

    #[test]
    fn messages_follow_pydantic_wording() {
        assert_eq!(expected_literals(&["codex"]), "'codex'");
        assert_eq!(expected_literals(&["codex", "claude-code"]), "'codex' or 'claude-code'");
        assert_eq!(expected_literals(&["A", "B", "C"]), "'A', 'B' or 'C'");
        let mut errors = Errors::new();
        let loc = Loc::header("idempotency-key");
        check_text(&mut errors, &loc, "", Text::between(1, 55));
        check_text(&mut errors, &loc, &"x".repeat(56), Text::between(1, 55));
        check_secret(&mut errors, &loc, "", Text::min(1));
        let messages: Vec<Value> =
            errors.into_vec().iter().map(|error| error["msg"].clone()).collect();
        assert_eq!(
            messages,
            [
                json!("String should have at least 1 character"),
                json!("String should have at most 55 characters"),
                json!("Value should have at least 1 item after validation, not 0"),
            ]
        );
    }
}
