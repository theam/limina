//! Errors, time and identifiers shared by every Limina crate.

pub mod error;
pub mod principal;
pub mod time;

pub use error::{Cause, ErrorKind, LiminaError, Result, internal};
pub use principal::{Principal, normalize_email};
pub use time::{Timestamp, iso, iso_opt, now};

/// A new random UUID in its canonical hyphenated form.
pub fn new_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}
