//! Offset pagination with the opaque cursors the Python runtime issued.
//!
//! A cursor is the next offset in decimal, base64url-encoded without padding (`"MQ"` is offset
//! 1). Decoding follows Python's `int(base64.urlsafe_b64decode(cursor + "===").decode("ascii"))`
//! exactly, including its leniency (characters outside the alphabet are skipped, data after the
//! padding is ignored, `+`/`/` are accepted), so every cursor a client holds keeps working.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use limina_core::{LiminaError, Result};
use serde::{Deserialize, Serialize};

/// The page size when the caller does not ask for one.
pub const DEFAULT_PAGE_LIMIT: i64 = 50;
/// The largest page any listing returns.
pub const MAX_PAGE_LIMIT: i64 = 200;

const INVALID_CURSOR: &str = "Pagination cursor is invalid.";
const INVALID_KNOWLEDGE_CURSOR: &str = "The pagination cursor is invalid.";

/// One page of a listing: `{"items", "next_cursor", "total"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    pub total: i64,
}

/// Clamps a requested page size to `1..=200`.
pub fn clamp_limit(limit: i64) -> i64 {
    limit.clamp(1, MAX_PAGE_LIMIT)
}

/// The offset a cursor points at (0 without a cursor). Negative offsets read as 0 and offsets
/// beyond `i64::MAX` saturate.
pub fn page_offset(cursor: Option<&str>) -> Result<i64> {
    decode(cursor, INVALID_CURSOR)
}

/// [`page_offset`] for knowledge queries, whose error message differs in the Python runtime.
pub(crate) fn knowledge_page_offset(cursor: Option<&str>) -> Result<i64> {
    decode(cursor, INVALID_KNOWLEDGE_CURSOR)
}

/// The cursor of the page after `count` items starting at `offset`, or `None` on the last page.
pub fn next_cursor(offset: i64, count: usize, total: i64) -> Option<String> {
    let next = offset.saturating_add(i64::try_from(count).unwrap_or(i64::MAX));
    (next < total).then(|| encode_cursor(next))
}

/// The cursor for an offset.
pub fn encode_cursor(offset: i64) -> String {
    URL_SAFE_NO_PAD.encode(offset.to_string())
}

fn decode(cursor: Option<&str>, message: &'static str) -> Result<i64> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    lenient_urlsafe_b64decode(cursor)
        .filter(|bytes| bytes.is_ascii())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|text| python_int(&text))
        .map(|value| value.max(0))
        .ok_or_else(|| LiminaError::invariant(message))
}

/// `base64.urlsafe_b64decode(text + "===")` with CPython's non-strict `a2b_base64` rules.
fn lenient_urlsafe_b64decode(text: &str) -> Option<Vec<u8>> {
    // Python rejects non-ASCII text before decoding.
    if !text.is_ascii() {
        return None;
    }
    let mut decoded = Vec::with_capacity(text.len());
    let mut quad_position = 0_u8;
    let mut left = 0_u8;
    let mut pads = 0_u8;
    for &byte in text.as_bytes().iter().chain(b"===") {
        if byte == b'=' {
            if quad_position >= 2 {
                pads += 1;
                if quad_position + pads >= 4 {
                    // A complete pad sequence ends the input; anything after it is ignored.
                    return Some(decoded);
                }
            }
            continue;
        }
        let Some(value) = sextet(byte) else {
            continue;
        };
        pads = 0;
        match quad_position {
            0 => {
                left = value;
                quad_position = 1;
            }
            1 => {
                decoded.push((left << 2) | (value >> 4));
                left = value & 0x0f;
                quad_position = 2;
            }
            2 => {
                decoded.push((left << 4) | (value >> 2));
                left = value & 0x03;
                quad_position = 3;
            }
            _ => {
                decoded.push((left << 6) | value);
                left = 0;
                quad_position = 0;
            }
        }
    }
    (quad_position == 0).then_some(decoded)
}

/// The value of one base64 character after the URL-safe translation (`-` → `+`, `_` → `/`).
fn sextet(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}

/// Python's `int(text)` for ASCII text: surrounding ASCII whitespace, an optional sign, and
/// decimal digits with single underscores between them. Saturates at the `i64` bounds.
fn python_int(text: &str) -> Option<i64> {
    let trimmed = text.trim_matches([' ', '\t', '\n', '\u{0b}', '\u{0c}', '\r']);
    let (negative, digits) = match trimmed.as_bytes().first() {
        Some(b'-') => (true, &trimmed[1..]),
        Some(b'+') => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    let mut value: i64 = 0;
    let mut previous_digit = false;
    let mut any_digit = false;
    for byte in digits.bytes() {
        match byte {
            b'0'..=b'9' => {
                let digit = i64::from(byte - b'0');
                value = value.saturating_mul(10).saturating_add(digit);
                previous_digit = true;
                any_digit = true;
            }
            b'_' if previous_digit => previous_digit = false,
            _ => return None,
        }
    }
    if !any_digit || !previous_digit {
        return None;
    }
    Some(if negative { -value } else { value })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_decode_like_python() {
        // Expected values produced by the Python runtime's `_page_offset`.
        let cases: &[(&str, Option<i64>)] = &[
            ("MQ", Some(1)),
            ("MTA", Some(10)),
            ("NTA=", Some(50)),
            ("NTA===", Some(50)),
            ("", None),
            ("M", None),
            ("MQ!", Some(1)),
            ("M Q", Some(1)),
            ("-1", None),
            ("LTE", Some(0)),
            ("IDUg", Some(5)),
            ("MV8w", Some(10)),
            ("MF9f", None),
            ("AA", None),
            ("_w", None),
            ("/w", None),
            ("+w", None),
            ("MQ=MQ", None),
            ("MTIz\n", Some(123)),
            ("é", None),
            ("MA", Some(0)),
            ("4oCDMQ", None),
            ("HDE", None),
            ("MQ=", Some(1)),
            ("M==Q", Some(1)),
            ("MTI=MTI", Some(12)),
            ("MTIzNA", Some(1234)),
            ("MTIzNA=x", Some(1234)),
            ("MQ==MQ", Some(1)),
            ("K1E", None),
            ("ICAxMiAg", Some(12)),
            ("OTk5OTk5OTk5OTk5OTk5OTk5OTk5", Some(i64::MAX)),
        ];
        for (cursor, expected) in cases {
            let decoded = page_offset(Some(cursor));
            match expected {
                Some(offset) => assert_eq!(decoded.unwrap(), *offset, "cursor {cursor:?}"),
                None => {
                    assert_eq!(decoded.unwrap_err().message, INVALID_CURSOR, "cursor {cursor:?}");
                }
            }
        }
        assert_eq!(page_offset(None).unwrap(), 0);
        assert_eq!(knowledge_page_offset(Some("M")).unwrap_err().message, INVALID_KNOWLEDGE_CURSOR);
    }

    #[test]
    fn next_cursors_encode_like_python() {
        assert_eq!(next_cursor(0, 1, 2).as_deref(), Some("MQ"));
        assert_eq!(next_cursor(0, 2, 2), None);
        assert_eq!(next_cursor(5, 3, 100).as_deref(), Some("OA"));
        assert_eq!(next_cursor(123, 77, 1000).as_deref(), Some("MjAw"));
        assert_eq!(next_cursor(i64::MAX, 0, 3), None);
        assert_eq!(clamp_limit(0), 1);
        assert_eq!(clamp_limit(500), 200);
    }
}
