//! UTC timestamps with the same wire formats as the Python runtime.

use chrono::{DateTime, SecondsFormat, SubsecRound, Utc};

pub type Timestamp = DateTime<Utc>;

/// The current time, truncated to microseconds (the precision both databases store).
pub fn now() -> Timestamp {
    Utc::now().trunc_subsecs(6)
}

/// ISO-8601 with an explicit `+00:00` offset, omitting a zero fraction, like Python's
/// `datetime.isoformat()` for aware UTC values.
pub fn iso(value: &Timestamp) -> String {
    let value = value.trunc_subsecs(6);
    let format = if value.timestamp_subsec_micros() == 0 {
        SecondsFormat::Secs
    } else {
        SecondsFormat::Micros
    };
    value.to_rfc3339_opts(format, false)
}

pub fn iso_opt(value: Option<&Timestamp>) -> Option<String> {
    value.map(iso)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn iso_matches_python_isoformat() {
        let whole = Utc.with_ymd_and_hms(2026, 10, 15, 12, 0, 0).unwrap();
        assert_eq!(iso(&whole), "2026-10-15T12:00:00+00:00");
        let fractional = whole + chrono::Duration::microseconds(1234);
        assert_eq!(iso(&fractional), "2026-10-15T12:00:00.001234+00:00");
    }
}

/// `#[serde(with = "limina_core::time::serde_iso")]`: Python-compatible ISO strings on the wire.
pub mod serde_iso {
    use super::{Timestamp, iso};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &Timestamp, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&iso(value))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Timestamp, D::Error> {
        let text = String::deserialize(deserializer)?;
        super::parse(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid timestamp: {text}")))
    }
}

/// `#[serde(with = "limina_core::time::serde_iso_opt")]` for optional timestamps.
pub mod serde_iso_opt {
    use super::{Timestamp, iso};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        value: &Option<Timestamp>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(value) => serializer.serialize_str(&iso(value)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Timestamp>, D::Error> {
        match Option::<String>::deserialize(deserializer)? {
            None => Ok(None),
            Some(text) => super::parse(&text)
                .map(Some)
                .ok_or_else(|| serde::de::Error::custom(format!("invalid timestamp: {text}"))),
        }
    }
}

/// Parses RFC 3339 / ISO-8601 timestamps, treating values without an offset as UTC.
pub fn parse(text: &str) -> Option<Timestamp> {
    if let Ok(value) = DateTime::parse_from_rfc3339(text) {
        return Some(value.with_timezone(&Utc));
    }
    for format in
        ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S"]
    {
        if let Ok(value) = chrono::NaiveDateTime::parse_from_str(text, format) {
            return Some(value.and_utc());
        }
    }
    None
}
