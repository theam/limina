//! Persistence for the Limina runtime.
//!
//! The runtime supports SQLite (single node, the default) and PostgreSQL (shared deployments) with
//! one schema, compatible with databases created by the Python runtime. Queries are written once
//! in a common dialect with `$N` placeholders and run on either backend through [`Tx`].
//!
//! Conventions that keep one query portable:
//!
//! - `INTEGER` columns decode as `i32` and `BIGINT` columns as `i64` (PostgreSQL is strict about
//!   integer widths). Cast integer aggregates explicitly: `CAST(COUNT(*) AS BIGINT)`.
//! - Timestamps bind as [`Arg::Time`] and decode as `chrono::DateTime<Utc>`. SQLite stores them as
//!   `YYYY-MM-DD HH:MM:SS.ffffff` text in UTC, exactly like SQLAlchemy did, so text comparisons
//!   order correctly.
//! - JSON columns bind as [`Arg::Json`] and decode as `sqlx::types::Json<serde_json::Value>`.
//! - Row locks: append [`Tx::for_update`] to a `SELECT`; it is empty on SQLite, where write
//!   transactions take the database lock up front instead.

pub mod cipher;
pub mod db;
pub mod models;
pub mod schema;

pub use cipher::SecretCipher;
pub use db::{Arg, Db, DbError, Tx};
pub use sqlx::types::Json;

/// Builds a `Vec<Arg>` from values that convert into [`Arg`].
#[macro_export]
macro_rules! args {
    () => { ::std::vec::Vec::<$crate::Arg>::new() };
    ($($value:expr),+ $(,)?) => { vec![$($crate::Arg::from($value)),+] };
}
