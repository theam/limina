//! Validation rules of the collaboration and knowledge surfaces, with the exact semantics of the
//! Python runtime: Python's `str.strip()`, `urllib.parse.urlsplit`/`parse_qsl`, and sizes measured
//! as the length of `json.dumps(value, ensure_ascii=False)`.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::OnceLock;

use limina_core::{LiminaError, Result, internal};
use regex::Regex;
use serde_json::{Map, Number, Value};

/// The longest knowledge tag, in characters.
pub const TAG_LIMIT: usize = 80;
/// The largest source metadata document, in `json.dumps` characters.
pub const SOURCE_METADATA_LIMIT: usize = 16_384;
/// The largest saved-view query, in `json.dumps` characters.
pub const SAVED_VIEW_QUERY_LIMIT: usize = 8192;

const SENSITIVE_METADATA_KEYS: &[&str] =
    &["secret", "token", "password", "credential", "api_key", "apikey"];
const METADATA_REFERENCE_SUFFIXES: &[&str] = &["_env", "_secret_name", "_secret_ref"];
const SENSITIVE_QUERY_KEYS: &[&str] = &[
    "secret",
    "token",
    "password",
    "credential",
    "api_key",
    "apikey",
    "signature",
    "sig",
    "authorization",
    "auth",
];

/// Characters whose NFKC form contains `/`, `?`, `#`, `@` or `:` (Unicode 15.1, as listed by
/// Python's `unicodedata`). `urlsplit` rejects a netloc containing any of them.
const NFKC_DELIMITER_CHARACTERS: &[char] = &[
    '\u{2047}', '\u{2048}', '\u{2049}', '\u{2100}', '\u{2101}', '\u{2105}', '\u{2106}', '\u{2A74}',
    '\u{FE13}', '\u{FE16}', '\u{FE55}', '\u{FE56}', '\u{FE5F}', '\u{FE6B}', '\u{FF03}', '\u{FF0F}',
    '\u{FF1A}', '\u{FF1F}', '\u{FF20}',
];

/// Python's `str.strip()`: Unicode whitespace plus the ASCII separators U+001C–U+001F.
pub fn py_strip(value: &str) -> &str {
    value.trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}

/// The stripped value, or `"<Label> cannot be empty."` (the label capitalized like Python's
/// `str.capitalize()`, so `"source URI"` reads `"Source uri"`).
pub fn required(label: &str, value: &str) -> Result<String> {
    let stripped = py_strip(value);
    if stripped.is_empty() {
        return Err(LiminaError::invariant(format!("{} cannot be empty.", capitalize(label))));
    }
    Ok(stripped.to_owned())
}

fn capitalize(label: &str) -> String {
    let lower = label.to_lowercase();
    let mut chars = lower.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// A knowledge tag in its stored form: trimmed, lowercase, spaces as hyphens, at most 80
/// characters of letters, numbers, dots, underscores or hyphens.
pub fn normalize_tag(value: &str) -> Result<String> {
    let tag = py_strip(value).to_lowercase().replace(' ', "-");
    if tag.is_empty() || tag.chars().count() > TAG_LIMIT || !tag_pattern()?.is_match(&tag) {
        return Err(LiminaError::invariant(
            "Knowledge tags must use letters, numbers, dots, underscores, or hyphens.",
        ));
    }
    Ok(tag)
}

/// Python's `str.isalnum()` is "letter or number" (general categories L* and N*).
fn tag_pattern() -> Result<&'static Regex> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    if let Some(pattern) = PATTERN.get() {
        return Ok(pattern);
    }
    let compiled = Regex::new(r"^[\p{L}\p{N}_.\-]+$").map_err(internal)?;
    Ok(PATTERN.get_or_init(|| compiled))
}

/// Rejects oversized source metadata and keys that look like credential values. Keys ending in
/// `_env`, `_secret_name` or `_secret_ref` reference a project secret and are allowed.
pub fn validate_source_metadata(metadata: &Map<String, Value>) -> Result<()> {
    if python_json_object_len(metadata) > SOURCE_METADATA_LIMIT {
        return Err(LiminaError::invariant("Source metadata must be at most 16384 characters."));
    }
    inspect_metadata_object(metadata)
}

fn inspect_metadata_object(object: &Map<String, Value>) -> Result<()> {
    for (raw_key, item) in object {
        let key = raw_key.to_lowercase();
        let is_reference = METADATA_REFERENCE_SUFFIXES.iter().any(|suffix| key.ends_with(suffix));
        if SENSITIVE_METADATA_KEYS.iter().any(|fragment| key.contains(fragment)) && !is_reference {
            return Err(LiminaError::invariant(
                "Source metadata cannot contain credential values; reference a write-only \
                 project secret by environment-variable name.",
            ));
        }
        inspect_metadata_value(item)?;
    }
    Ok(())
}

fn inspect_metadata_value(value: &Value) -> Result<()> {
    match value {
        Value::Object(object) => inspect_metadata_object(object),
        Value::Array(items) => items.iter().try_for_each(inspect_metadata_value),
        _ => Ok(()),
    }
}

/// Rejects source URIs that embed credentials. `URL` sources must also be absolute HTTP(S) URLs.
pub fn validate_source_uri(uri: &str, require_http: bool) -> Result<()> {
    // Python raised an unhandled `ValueError` here (an internal error); report it as invalid input.
    let parsed = split_url(uri).map_err(|reason| {
        LiminaError::invariant("Source URI is invalid.").with("reason", reason)
    })?;
    if require_http
        && (!matches!(parsed.scheme.as_str(), "http" | "https")
            || hostname(&parsed.netloc).is_none())
    {
        return Err(LiminaError::invariant("URL sources must use an absolute HTTP or HTTPS URL."));
    }
    let (username, password) = userinfo(&parsed.netloc);
    if username.is_some_and(|value| !value.is_empty())
        || password.is_some_and(|value| !value.is_empty())
    {
        return Err(LiminaError::invariant(
            "Source URIs cannot embed credentials; configure a write-only project secret.",
        ));
    }
    if query_keys(&parsed.query).iter().any(|key| is_sensitive_query_key(key)) {
        return Err(LiminaError::invariant(
            "Source URIs cannot embed credential query parameters; configure a write-only \
             project secret.",
        ));
    }
    Ok(())
}

fn is_sensitive_query_key(key: &str) -> bool {
    let normalized = key.to_lowercase().replace('-', "_");
    SENSITIVE_QUERY_KEYS.iter().any(|fragment| {
        normalized == *fragment
            || normalized.strip_suffix(fragment).is_some_and(|prefix| prefix.ends_with('_'))
    })
}

/// The parts of `urllib.parse.urlsplit` the source rules use.
#[derive(Debug, Default, PartialEq, Eq)]
struct SplitUrl {
    scheme: String,
    netloc: String,
    query: String,
}

/// `urllib.parse.urlsplit` (Python 3.13), returning the reason where Python raised `ValueError`.
fn split_url(url: &str) -> std::result::Result<SplitUrl, &'static str> {
    let url: String = url
        .trim_start_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\r' | '\n'))
        .collect();
    let mut rest = url.as_str();
    let mut split = SplitUrl::default();
    if let Some(colon) = rest.find(':')
        && rest.bytes().next().is_some_and(|first| first.is_ascii_alphabetic())
        && rest[..colon]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    {
        split.scheme = rest[..colon].to_ascii_lowercase();
        rest = &rest[colon + 1..];
    }
    if let Some(after) = rest.strip_prefix("//") {
        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        let netloc = &after[..end];
        rest = &after[end..];
        let (open, close) = (netloc.contains('['), netloc.contains(']'));
        if open != close {
            return Err("Invalid IPv6 URL");
        }
        if open {
            let bracketed = netloc.split_once('[').map_or("", |(_, after)| after);
            check_bracketed_host(bracketed.split_once(']').map_or(bracketed, |(host, _)| host))?;
        }
        split.netloc = netloc.to_owned();
    }
    if let Some((before, _fragment)) = rest.split_once('#') {
        rest = before;
    }
    if let Some((_, query)) = rest.split_once('?') {
        split.query = query.to_owned();
    }
    check_netloc(&split.netloc)?;
    Ok(split)
}

/// `_check_bracketed_host`: an IPvFuture literal or an IPv6 address (never IPv4) in brackets.
fn check_bracketed_host(host: &str) -> std::result::Result<(), &'static str> {
    if let Some(future) = host.strip_prefix('v') {
        // \Av[a-fA-F0-9]+\..+\Z
        let hex_digits = future.bytes().take_while(u8::is_ascii_hexdigit).count();
        let tail = future[hex_digits..].strip_prefix('.');
        return match tail {
            Some(tail) if hex_digits > 0 && !tail.is_empty() && !tail.contains('\n') => Ok(()),
            _ => Err("IPvFuture address is invalid"),
        };
    }
    if host.parse::<Ipv4Addr>().is_ok() {
        return Err("An IPv4 address cannot be in brackets");
    }
    let address = match host.split_once('%') {
        None => host,
        Some((address, zone)) if !zone.is_empty() && !zone.contains('%') => address,
        Some(_) => return Err("Invalid IPv6 address"),
    };
    address.parse::<Ipv6Addr>().map(|_| ()).map_err(|_| "Invalid IPv6 address")
}

/// `_checknetloc`: no characters that NFKC-normalize into URL delimiters.
fn check_netloc(netloc: &str) -> std::result::Result<(), &'static str> {
    if netloc.is_ascii() || !netloc.chars().any(|c| NFKC_DELIMITER_CHARACTERS.contains(&c)) {
        return Ok(());
    }
    Err("The netloc contains invalid characters under NFKC normalization")
}

/// `SplitResult.hostname` (only whether it is present matters here).
fn hostname(netloc: &str) -> Option<&str> {
    let host_info = netloc.rsplit_once('@').map_or(netloc, |(_, host)| host);
    let host = match host_info.split_once('[') {
        Some((_, bracketed)) => bracketed.split_once(']').map_or(bracketed, |(host, _)| host),
        None => host_info.split_once(':').map_or(host_info, |(host, _)| host),
    };
    (!host.is_empty()).then_some(host)
}

/// `SplitResult.username` and `SplitResult.password`.
fn userinfo(netloc: &str) -> (Option<&str>, Option<&str>) {
    match netloc.rsplit_once('@') {
        None => (None, None),
        Some((info, _)) => match info.split_once(':') {
            Some((username, password)) => (Some(username), Some(password)),
            None => (Some(info), None),
        },
    }
}

/// The names `urllib.parse.parse_qsl(query)` returns: `&`-separated fields with a `=` and a
/// non-empty value, `+` read as a space and percent-escapes decoded.
fn query_keys(query: &str) -> Vec<String> {
    query
        .split('&')
        .filter_map(|field| field.split_once('='))
        .filter(|(_, value)| !value.is_empty())
        .map(|(name, _)| unquote(&name.replace('+', " ")))
        .collect()
}

/// `urllib.parse.unquote` with UTF-8 and `errors="replace"`: escapes are decoded within each run
/// of ASCII characters, and other characters are kept as they are.
fn unquote(text: &str) -> String {
    if !text.contains('%') {
        return text.to_owned();
    }
    let mut result = String::with_capacity(text.len());
    let mut ascii_run = Vec::new();
    for character in text.chars() {
        if character.is_ascii() {
            ascii_run.push(character as u8);
        } else {
            result.push_str(&percent_decode(&ascii_run));
            ascii_run.clear();
            result.push(character);
        }
    }
    result.push_str(&percent_decode(&ascii_run));
    result
}

fn percent_decode(ascii: &[u8]) -> String {
    let hex = |byte: Option<&u8>| byte.and_then(|b| char::from(*b).to_digit(16));
    let mut bytes = Vec::with_capacity(ascii.len());
    let mut rest = ascii;
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'%'
            && let (Some(high), Some(low)) = (hex(tail.first()), hex(tail.get(1)))
            && let Ok(decoded) = u8::try_from(high * 16 + low)
        {
            bytes.push(decoded);
            rest = &tail[2..];
        } else {
            bytes.push(byte);
            rest = tail;
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The length in characters of Python's `json.dumps(value, ensure_ascii=False)`.
pub fn python_json_len(value: &Value) -> usize {
    match value {
        Value::Null | Value::Bool(true) => 4,
        Value::Bool(false) => 5,
        Value::Number(number) => python_number_repr(number).chars().count(),
        Value::String(text) => python_json_string_len(text),
        Value::Array(items) => {
            2 + items.iter().map(python_json_len).sum::<usize>() + 2 * items.len().saturating_sub(1)
        }
        Value::Object(object) => python_json_object_len(object),
    }
}

fn python_json_object_len(object: &Map<String, Value>) -> usize {
    2 + object
        .iter()
        .map(|(key, value)| python_json_string_len(key) + 2 + python_json_len(value))
        .sum::<usize>()
        + 2 * object.len().saturating_sub(1)
}

fn python_json_string_len(text: &str) -> usize {
    2 + text
        .chars()
        .map(|c| match c {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{08}' | '\u{0c}' => 2,
            c if c < ' ' => 6,
            _ => 1,
        })
        .sum::<usize>()
}

/// `repr()` of a JSON number as Python prints it (`1e+16`, `1e-05`, `0.0001`, `-0.0`).
fn python_number_repr(number: &Number) -> String {
    if let Some(value) = number.as_i64() {
        return value.to_string();
    }
    if let Some(value) = number.as_u64() {
        return value.to_string();
    }
    number.as_f64().map(python_float_repr).unwrap_or_else(|| number.to_string())
}

fn python_float_repr(value: f64) -> String {
    if !value.is_finite() {
        return if value.is_nan() {
            "NaN".to_owned()
        } else if value > 0.0 {
            "Infinity".to_owned()
        } else {
            "-Infinity".to_owned()
        };
    }
    let sign = if value.is_sign_negative() { "-" } else { "" };
    // `{:e}` prints the shortest round-tripping digits, like Python's repr.
    let scientific = format!("{:e}", value.abs());
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((scientific.as_str(), "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let point = exponent + 1;
    if digits == "0" {
        return format!("{sign}0.0");
    }
    if !(-3..=16).contains(&point) {
        let (first, rest) = digits.split_at(1);
        let fraction = if rest.is_empty() { String::new() } else { format!(".{rest}") };
        let exponent_sign = if exponent < 0 { '-' } else { '+' };
        return format!("{sign}{first}{fraction}e{exponent_sign}{:02}", exponent.abs());
    }
    let length = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    if point <= 0 {
        let zeros = "0".repeat(usize::try_from(-point).unwrap_or(0));
        format!("{sign}0.{zeros}{digits}")
    } else if point >= length {
        let zeros = "0".repeat(usize::try_from(point - length).unwrap_or(0));
        format!("{sign}{digits}{zeros}.0")
    } else {
        let (whole, fraction) = digits.split_at(usize::try_from(point).unwrap_or(0));
        format!("{sign}{whole}.{fraction}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn required_text_uses_python_capitalization() {
        assert_eq!(
            required("source URI", "  ").unwrap_err().message,
            "Source uri cannot be empty."
        );
        assert_eq!(
            required("success criteria", "").unwrap_err().message,
            "Success criteria cannot be empty."
        );
        assert_eq!(required("name", "\u{1c} Ada \u{3000}").unwrap(), "Ada");
        assert!(required("comment", "\u{200b}").is_ok(), "U+200B is not whitespace in Python");
    }

    #[test]
    fn tags_follow_python_isalnum() {
        // Expected values produced by the Python runtime's `_normalize_tag`.
        let accepted = [
            ("Hello World", "hello-world"),
            ("ℌ", "ℌ"),
            ("一二", "一二"),
            ("  x  ", "x"),
            ("²", "²"),
            ("Ⅻ", "ⅻ"),
            ("v1.2_rc-3", "v1.2_rc-3"),
        ];
        for (input, expected) in accepted {
            assert_eq!(normalize_tag(input).unwrap(), expected, "tag {input:?}");
        }
        for rejected in ["Ⓐ", "हिंदी", "a\u{1c}b", "a/b", &"x".repeat(81), "İ", "\u{301}", "", "  "]
        {
            assert!(normalize_tag(rejected).is_err(), "tag {rejected:?}");
        }
        assert_eq!(normalize_tag(&"é".repeat(80)).unwrap().chars().count(), 80);
    }

    #[test]
    fn source_uris_follow_urlsplit() {
        const QUERY: &str = "Source URIs cannot embed credential query parameters; configure a \
                             write-only project secret.";
        const USERINFO: &str =
            "Source URIs cannot embed credentials; configure a write-only project secret.";
        const HTTP: &str = "URL sources must use an absolute HTTP or HTTPS URL.";
        const INVALID: &str = "Source URI is invalid.";
        // Expected outcomes produced by the Python runtime's `_validate_source_uri`.
        let cases: &[(&str, bool, Option<&str>)] = &[
            ("https://example.test/benchmark", true, None),
            ("https://example.test/data?access_token=secret", true, Some(QUERY)),
            ("github://user:token@example.test/repository", false, Some(USERINFO)),
            ("ftp://example.test/x", true, Some(HTTP)),
            ("https:///path", true, Some(HTTP)),
            ("HTTPS://Example.test", true, None),
            ("https://@example.test", true, None),
            ("https://:pw@example.test", true, Some(USERINFO)),
            ("https://example.test/?Api-Key=1", true, Some(QUERY)),
            ("https://example.test/?x_auth=1", true, Some(QUERY)),
            ("https://example.test/?auth_x=1", true, None),
            ("https://example.test/?token=", true, None),
            ("https://example.test/?token", true, None),
            ("https://example.test/?to%6Ben=1", true, Some(QUERY)),
            ("https://example.test/?my+token=1", true, None),
            ("https://example.test/?my_sig=1&b=2", true, Some(QUERY)),
            ("https://example.test/#?token=1", true, None),
            ("https://[::1/x", true, Some(INVALID)),
            ("https://example.test]/x", true, Some(INVALID)),
            ("  https://example.test", true, None),
            ("ht\ttps://exa\nmple.test", true, None),
            ("1http://example.test", true, Some(HTTP)),
            ("connector:thing", false, None),
            ("s3://bucket/key?X-Amz-Signature=abc", false, Some(QUERY)),
            ("https://exa%40mple.test", true, None),
            ("https://a@b@example.test", true, Some(USERINFO)),
            ("https://example.test:99999", true, None),
            ("https://[::1]:80/", true, None),
            ("https://[fe80::1%eth0]/", true, None),
            ("https://[127.0.0.1]/", true, Some(INVALID)),
            ("https://[v1.x]/", true, None),
            ("https://[vz.x]/", true, Some(INVALID)),
            ("https://example.test/?a=1;token=2", true, None),
            ("https://example.test/?%E9token=1", true, None),
            ("https://example.test/?x_TOKEN=1", true, Some(QUERY)),
            ("https://\u{ff0f}example.test", true, Some(INVALID)),
            ("https://ex\u{2100}ample.test", true, Some(INVALID)),
            ("https://ex\u{e9}mple.test", true, None),
        ];
        for (uri, require_http, expected) in cases {
            let outcome = validate_source_uri(uri, *require_http).err().map(|e| e.message);
            assert_eq!(outcome.as_deref(), *expected, "uri {uri:?}");
        }
    }

    #[test]
    fn source_metadata_rejects_credentials() {
        let rejected = "Source metadata cannot contain credential values; reference a write-only \
                        project secret by environment-variable name.";
        let cases = [
            (json!({"purpose": "evaluation"}), None),
            (json!({"api_key_env": "X"}), None),
            (json!({"nested": [{"Password": "x"}]}), Some(rejected)),
            (json!({"token_secret_ref": "X"}), None),
            (json!({"tokens": 1}), Some(rejected)),
            (json!({"x": "secret"}), None),
            (json!({"a": {"b": {"credentials_secret_name": 1}}}), None),
            (json!({"Auth": 1}), None),
            (json!({"x": "y".repeat(16_370)}), None),
            (json!({"x": "é".repeat(16_370)}), None),
            (
                json!({"x": "é".repeat(16_376)}),
                Some("Source metadata must be at most 16384 characters."),
            ),
        ];
        for (metadata, expected) in cases {
            let Value::Object(metadata) = metadata else { unreachable!() };
            let outcome = validate_source_metadata(&metadata).err().map(|e| e.message);
            assert_eq!(outcome.as_deref(), expected);
        }
    }

    #[test]
    fn json_sizes_match_python_dumps() {
        // json.dumps({...}, ensure_ascii=False) in Python 3.13.
        let value =
            json!({"a": [1, 2.5, 1e16, 1e-5, 0.0001, -0.0, 1e22, "é\n\u{7f}\u{2028}\u{1}"]});
        let python =
            "{\"a\": [1, 2.5, 1e+16, 1e-05, 0.0001, -0.0, 1e+22, \"é\\n\u{7f}\u{2028}\\u0001\"]}";
        assert_eq!(python_json_len(&value), python.chars().count());
        assert_eq!(python_json_len(&json!({"purpose": "evaluation"})), 25);
        assert_eq!(python_json_len(&json!({})), 2);
        assert_eq!(python_json_len(&json!([])), 2);
        for (value, repr) in [
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (1e-5, "1e-05"),
            (0.0001, "0.0001"),
            (123.456, "123.456"),
            (1.2345678901234568e29, "1.2345678901234568e+29"),
            (5e-324, "5e-324"),
            (0.0, "0.0"),
            (-2.5, "-2.5"),
            (100.0, "100.0"),
        ] {
            assert_eq!(python_float_repr(value), repr);
        }
    }
}
