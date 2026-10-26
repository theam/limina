//! One connection pool and transaction type over SQLite and PostgreSQL.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use limina_core::{Cause, LiminaError, Timestamp};
use serde_json::Value;
use sqlx::postgres::{PgPoolOptions, PgRow};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow};
use sqlx::{FromRow, PgPool, Postgres, Sqlite, SqlitePool, Transaction};

/// A database failure. Converts into a [`LiminaError`] at the domain boundary while keeping the
/// details callers need to react (such as unique-constraint violations on idempotent commands).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct DbError(#[from] pub sqlx::Error);

impl DbError {
    pub fn is_unique_violation(&self) -> bool {
        matches!(&self.0, sqlx::Error::Database(error) if error.is_unique_violation())
    }

    pub fn is_foreign_key_violation(&self) -> bool {
        matches!(&self.0, sqlx::Error::Database(error) if error.is_foreign_key_violation())
    }

    pub fn is_row_not_found(&self) -> bool {
        matches!(self.0, sqlx::Error::RowNotFound)
    }
}

impl From<DbError> for LiminaError {
    fn from(error: DbError) -> Self {
        if error.is_unique_violation() || error.is_foreign_key_violation() {
            tracing::debug!(error = %error.0, "constraint violation");
            return LiminaError::conflict("The command conflicted with another concurrent write.")
                .caused_by(Cause::Constraint);
        }
        tracing::error!(error = %error.0, "database error");
        LiminaError::transport("The runtime database is unavailable.")
            .with_code("database_error")
            .caused_by(Cause::Database)
    }
}

pub type DbResult<T> = Result<T, DbError>;

/// A bound query parameter.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Text(Option<String>),
    Int(Option<i64>),
    Int32(Option<i32>),
    Float(Option<f64>),
    Bool(Option<bool>),
    Json(Option<Value>),
    Time(Option<Timestamp>),
}

macro_rules! arg_from {
    ($variant:ident, $type:ty, $convert:expr) => {
        impl From<$type> for Arg {
            fn from(value: $type) -> Self {
                Arg::$variant(Some($convert(value)))
            }
        }
        impl From<Option<$type>> for Arg {
            fn from(value: Option<$type>) -> Self {
                Arg::$variant(value.map($convert))
            }
        }
    };
}

arg_from!(Text, String, |v| v);
impl From<&str> for Arg {
    fn from(value: &str) -> Self {
        Arg::Text(Some(value.to_owned()))
    }
}

impl From<&String> for Arg {
    fn from(value: &String) -> Self {
        Arg::Text(Some(value.clone()))
    }
}
arg_from!(Int, i64, |v| v);
arg_from!(Int, u64, |v: u64| i64::try_from(v).unwrap_or(i64::MAX));
arg_from!(Int, usize, |v: usize| i64::try_from(v).unwrap_or(i64::MAX));
arg_from!(Int32, i32, |v| v);
arg_from!(Float, f64, |v| v);
arg_from!(Bool, bool, |v| v);
arg_from!(Json, Value, |v| v);
arg_from!(Json, &Value, |v: &Value| v.clone());
arg_from!(Time, Timestamp, |v| v);
arg_from!(Time, &Timestamp, |v: &Timestamp| *v);

impl From<Option<&str>> for Arg {
    fn from(value: Option<&str>) -> Self {
        Arg::Text(value.map(str::to_owned))
    }
}

impl From<Option<&String>> for Arg {
    fn from(value: Option<&String>) -> Self {
        Arg::Text(value.cloned())
    }
}

/// Query text. Every statement is a constant written in this codebase with `$N` placeholders;
/// values always travel as bound parameters, never interpolated into the text.
fn sql_text(sql: &str) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(sql.to_owned())
}

/// SQLAlchemy's SQLite storage format, kept so existing rows and new rows compare as text.
fn sqlite_time(value: &Timestamp) -> String {
    value.naive_utc().format("%Y-%m-%d %H:%M:%S%.6f").to_string()
}

/// A JSON value bound as PostgreSQL `json`. sqlx binds `Json<T>` as `jsonb`, which a `json`
/// column stores as `jsonb`'s canonical text with object keys re-sorted; the Python runtime
/// stored the text as written, and stored payloads and receipts keep their key order.
struct PgJson(Value);

impl sqlx::Type<Postgres> for PgJson {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        sqlx::postgres::PgTypeInfo::with_oid(sqlx::postgres::types::Oid(114))
    }
}

impl sqlx::Encode<'_, Postgres> for PgJson {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        // `json` travels as its text in both wire formats.
        serde_json::to_writer(&mut **buf, &self.0)?;
        Ok(sqlx::encode::IsNull::No)
    }
}

fn bind_sqlite<'q>(
    mut query: sqlx::query::Query<'q, Sqlite, <Sqlite as sqlx::Database>::Arguments>,
    args: &[Arg],
) -> sqlx::query::Query<'q, Sqlite, <Sqlite as sqlx::Database>::Arguments> {
    for arg in args {
        query = match arg {
            Arg::Text(value) => query.bind(value.clone()),
            Arg::Int(value) => query.bind(*value),
            Arg::Int32(value) => query.bind(*value),
            Arg::Float(value) => query.bind(*value),
            Arg::Bool(value) => query.bind(*value),
            Arg::Json(value) => query.bind(value.as_ref().map(Value::to_string)),
            Arg::Time(value) => query.bind(value.as_ref().map(sqlite_time)),
        };
    }
    query
}

fn bind_postgres<'q>(
    mut query: sqlx::query::Query<'q, Postgres, <Postgres as sqlx::Database>::Arguments>,
    args: &[Arg],
) -> sqlx::query::Query<'q, Postgres, <Postgres as sqlx::Database>::Arguments> {
    for arg in args {
        query = match arg {
            Arg::Text(value) => query.bind(value.clone()),
            Arg::Int(value) => query.bind(*value),
            Arg::Int32(value) => query.bind(*value),
            Arg::Float(value) => query.bind(*value),
            Arg::Bool(value) => query.bind(*value),
            Arg::Json(value) => query.bind(value.clone().map(PgJson)),
            Arg::Time(value) => query.bind(*value),
        };
    }
    query
}

fn bind_sqlite_as<'q, T>(
    mut query: sqlx::query::QueryAs<'q, Sqlite, T, <Sqlite as sqlx::Database>::Arguments>,
    args: &[Arg],
) -> sqlx::query::QueryAs<'q, Sqlite, T, <Sqlite as sqlx::Database>::Arguments> {
    for arg in args {
        query = match arg {
            Arg::Text(value) => query.bind(value.clone()),
            Arg::Int(value) => query.bind(*value),
            Arg::Int32(value) => query.bind(*value),
            Arg::Float(value) => query.bind(*value),
            Arg::Bool(value) => query.bind(*value),
            Arg::Json(value) => query.bind(value.as_ref().map(Value::to_string)),
            Arg::Time(value) => query.bind(value.as_ref().map(sqlite_time)),
        };
    }
    query
}

fn bind_postgres_as<'q, T>(
    mut query: sqlx::query::QueryAs<'q, Postgres, T, <Postgres as sqlx::Database>::Arguments>,
    args: &[Arg],
) -> sqlx::query::QueryAs<'q, Postgres, T, <Postgres as sqlx::Database>::Arguments> {
    for arg in args {
        query = match arg {
            Arg::Text(value) => query.bind(value.clone()),
            Arg::Int(value) => query.bind(*value),
            Arg::Int32(value) => query.bind(*value),
            Arg::Float(value) => query.bind(*value),
            Arg::Bool(value) => query.bind(*value),
            Arg::Json(value) => query.bind(value.clone().map(PgJson)),
            Arg::Time(value) => query.bind(*value),
        };
    }
    query
}

/// A row type that decodes from either backend. `#[derive(sqlx::FromRow)]` structs whose fields
/// follow the crate conventions implement it automatically.
pub trait Record:
    for<'r> FromRow<'r, SqliteRow> + for<'r> FromRow<'r, PgRow> + Send + Unpin
{
}
impl<T> Record for T where
    T: for<'r> FromRow<'r, SqliteRow> + for<'r> FromRow<'r, PgRow> + Send + Unpin
{
}

/// A single value that decodes from either backend (`i64`, `i32`, `String`, `bool`, …).
pub trait Scalar:
    for<'r> sqlx::Decode<'r, Sqlite>
    + sqlx::Type<Sqlite>
    + for<'r> sqlx::Decode<'r, Postgres>
    + sqlx::Type<Postgres>
    + Send
    + Unpin
{
}
impl<T> Scalar for T where
    T: for<'r> sqlx::Decode<'r, Sqlite>
        + sqlx::Type<Sqlite>
        + for<'r> sqlx::Decode<'r, Postgres>
        + sqlx::Type<Postgres>
        + Send
        + Unpin
{
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Sqlite,
    Postgres,
}

/// A connection pool for the configured backend.
#[derive(Debug, Clone)]
pub enum Db {
    Sqlite(SqlitePool),
    Postgres(PgPool),
}

impl Db {
    /// Connects to a database URL. Accepts the URL forms the Python runtime used
    /// (`sqlite:///relative.db`, `sqlite:////absolute.db`, `postgresql+psycopg://…`) as well as
    /// plain `sqlite:`/`postgres:` URLs.
    pub async fn connect(url: &str) -> DbResult<Self> {
        if let Some(path) = sqlite_path(url) {
            let options = match path {
                SqliteTarget::Memory => SqliteConnectOptions::from_str("sqlite::memory:")?,
                SqliteTarget::File(path) => {
                    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                        std::fs::create_dir_all(parent).map_err(sqlx::Error::Io)?;
                    }
                    SqliteConnectOptions::new().filename(path).create_if_missing(true)
                }
            }
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(30));
            let memory = matches!(sqlite_path(url), Some(SqliteTarget::Memory));
            let pool = SqlitePoolOptions::new()
                // An in-memory database lives in one connection.
                .max_connections(if memory { 1 } else { 8 })
                .min_connections(if memory { 1 } else { 0 })
                .idle_timeout(if memory { None } else { Some(Duration::from_secs(600)) })
                .max_lifetime(if memory { None } else { Some(Duration::from_secs(1800)) })
                .acquire_timeout(Duration::from_secs(30))
                .connect_with(options)
                .await?;
            return Ok(Self::Sqlite(pool));
        }
        let pool = PgPoolOptions::new()
            .max_connections(16)
            .acquire_timeout(Duration::from_secs(30))
            .test_before_acquire(true)
            .connect(&postgres_url(url))
            .await?;
        Ok(Self::Postgres(pool))
    }

    pub fn backend(&self) -> Backend {
        match self {
            Self::Sqlite(_) => Backend::Sqlite,
            Self::Postgres(_) => Backend::Postgres,
        }
    }

    /// A read transaction (or a write that tolerates a concurrent-writer retry).
    pub async fn begin(&self) -> DbResult<Tx> {
        Ok(match self {
            Self::Sqlite(pool) => Tx::Sqlite(pool.begin().await?),
            Self::Postgres(pool) => Tx::Postgres(pool.begin().await?),
        })
    }

    /// A write transaction. On SQLite it takes the write lock up front (`BEGIN IMMEDIATE`), so two
    /// writers queue on the busy timeout instead of failing when a read lock upgrades.
    pub async fn begin_write(&self) -> DbResult<Tx> {
        Ok(match self {
            Self::Sqlite(pool) => Tx::Sqlite(pool.begin_with("BEGIN IMMEDIATE").await?),
            Self::Postgres(pool) => Tx::Postgres(pool.begin().await?),
        })
    }

    pub async fn ping(&self) -> DbResult<()> {
        match self {
            Self::Sqlite(pool) => sqlx::query("SELECT 1").execute(pool).await.map(|_| ()),
            Self::Postgres(pool) => sqlx::query("SELECT 1").execute(pool).await.map(|_| ()),
        }
        .map_err(DbError)
    }

    pub async fn close(&self) {
        match self {
            Self::Sqlite(pool) => pool.close().await,
            Self::Postgres(pool) => pool.close().await,
        }
    }
}

enum SqliteTarget<'a> {
    Memory,
    File(&'a Path),
}

fn sqlite_path(url: &str) -> Option<SqliteTarget<'_>> {
    let rest = url.strip_prefix("sqlite:")?;
    let rest = rest.split('?').next().unwrap_or_default();
    if rest.is_empty() || rest == ":memory:" || rest == "//:memory:" || rest == "///:memory:" {
        return Some(SqliteTarget::Memory);
    }
    // SQLAlchemy: `sqlite:///relative` and `sqlite:////absolute`; sqlx: `sqlite://path`.
    let path = rest.strip_prefix("///").or_else(|| rest.strip_prefix("//")).unwrap_or(rest);
    Some(SqliteTarget::File(Path::new(path)))
}

fn postgres_url(url: &str) -> String {
    for prefix in ["postgresql+psycopg://", "postgresql+psycopg2://", "postgresql+asyncpg://"] {
        if let Some(rest) = url.strip_prefix(prefix) {
            return format!("postgres://{rest}");
        }
    }
    url.to_owned()
}

/// A transaction on either backend. Dropping it without [`Tx::commit`] rolls back.
#[derive(Debug)]
pub enum Tx {
    Sqlite(Transaction<'static, Sqlite>),
    Postgres(Transaction<'static, Postgres>),
}

impl Tx {
    pub fn backend(&self) -> Backend {
        match self {
            Self::Sqlite(_) => Backend::Sqlite,
            Self::Postgres(_) => Backend::Postgres,
        }
    }

    /// ` FOR UPDATE` on PostgreSQL, empty on SQLite.
    pub fn for_update(&self) -> &'static str {
        match self {
            Self::Sqlite(_) => "",
            Self::Postgres(_) => " FOR UPDATE",
        }
    }

    /// Executes a statement and returns the number of affected rows.
    pub async fn execute(&mut self, sql: &str, args: Vec<Arg>) -> DbResult<u64> {
        Ok(match self {
            Self::Sqlite(tx) => bind_sqlite(sqlx::query(sql_text(sql)), &args)
                .execute(&mut **tx)
                .await?
                .rows_affected(),
            Self::Postgres(tx) => bind_postgres(sqlx::query(sql_text(sql)), &args)
                .execute(&mut **tx)
                .await?
                .rows_affected(),
        })
    }

    pub async fn fetch_all<T: Record>(&mut self, sql: &str, args: Vec<Arg>) -> DbResult<Vec<T>> {
        Ok(match self {
            Self::Sqlite(tx) => {
                bind_sqlite_as(sqlx::query_as(sql_text(sql)), &args).fetch_all(&mut **tx).await?
            }
            Self::Postgres(tx) => {
                bind_postgres_as(sqlx::query_as(sql_text(sql)), &args).fetch_all(&mut **tx).await?
            }
        })
    }

    pub async fn fetch_optional<T: Record>(
        &mut self,
        sql: &str,
        args: Vec<Arg>,
    ) -> DbResult<Option<T>> {
        Ok(match self {
            Self::Sqlite(tx) => {
                bind_sqlite_as(sqlx::query_as(sql_text(sql)), &args)
                    .fetch_optional(&mut **tx)
                    .await?
            }
            Self::Postgres(tx) => {
                bind_postgres_as(sqlx::query_as(sql_text(sql)), &args)
                    .fetch_optional(&mut **tx)
                    .await?
            }
        })
    }

    pub async fn fetch_one<T: Record>(&mut self, sql: &str, args: Vec<Arg>) -> DbResult<T> {
        Ok(match self {
            Self::Sqlite(tx) => {
                bind_sqlite_as(sqlx::query_as(sql_text(sql)), &args).fetch_one(&mut **tx).await?
            }
            Self::Postgres(tx) => {
                bind_postgres_as(sqlx::query_as(sql_text(sql)), &args).fetch_one(&mut **tx).await?
            }
        })
    }

    /// The first column of the first row.
    pub async fn scalar<T: Scalar>(&mut self, sql: &str, args: Vec<Arg>) -> DbResult<T> {
        self.scalar_optional(sql, args).await?.ok_or(DbError(sqlx::Error::RowNotFound))
    }

    /// The first column of the first row, or `None` when there is no row.
    pub async fn scalar_optional<T: Scalar>(
        &mut self,
        sql: &str,
        args: Vec<Arg>,
    ) -> DbResult<Option<T>> {
        let row: Option<(T,)> = self.fetch_optional(sql, args).await?;
        Ok(row.map(|(value,)| value))
    }

    /// Runs several `;`-separated statements without parameters (schema scripts).
    pub async fn execute_script(&mut self, script: &str) -> DbResult<()> {
        for statement in split_statements(script) {
            match self {
                Self::Sqlite(tx) => {
                    sqlx::raw_sql(sqlx::AssertSqlSafe(statement)).execute(&mut **tx).await?;
                }
                Self::Postgres(tx) => {
                    sqlx::raw_sql(sqlx::AssertSqlSafe(statement)).execute(&mut **tx).await?;
                }
            }
        }
        Ok(())
    }

    pub async fn commit(self) -> DbResult<()> {
        match self {
            Self::Sqlite(tx) => tx.commit().await?,
            Self::Postgres(tx) => tx.commit().await?,
        }
        Ok(())
    }

    pub async fn rollback(self) -> DbResult<()> {
        match self {
            Self::Sqlite(tx) => tx.rollback().await?,
            Self::Postgres(tx) => tx.rollback().await?,
        }
        Ok(())
    }
}

/// Splits a schema script on `;` at line ends, dropping `--` comment lines.
fn split_statements(script: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    for line in script.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("--") {
            continue;
        }
        current.push_str(line);
        current.push('\n');
        if trimmed.ends_with(';') {
            let statement = current.trim().trim_end_matches(';').trim().to_owned();
            if !statement.is_empty() {
                statements.push(statement);
            }
            current.clear();
        }
    }
    let rest = current.trim().trim_end_matches(';').trim();
    if !rest.is_empty() {
        statements.push(rest.to_owned());
    }
    statements
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_urls_from_the_python_runtime_still_work() {
        assert!(
            matches!(sqlite_path("sqlite:///.limina/runtime.db"), Some(SqliteTarget::File(p)) if p == Path::new(".limina/runtime.db"))
        );
        assert!(
            matches!(sqlite_path("sqlite:////var/lib/limina/runtime.db"), Some(SqliteTarget::File(p)) if p == Path::new("/var/lib/limina/runtime.db"))
        );
        assert!(matches!(sqlite_path("sqlite::memory:"), Some(SqliteTarget::Memory)));
        assert!(sqlite_path("postgres://x").is_none());
        assert_eq!(postgres_url("postgresql+psycopg://u:p@h/db"), "postgres://u:p@h/db");
    }

    #[test]
    fn scripts_split_into_statements() {
        let parts =
            split_statements("-- c\nCREATE TABLE a (\n x INT\n);\n\nCREATE INDEX i ON a (x);\n");
        assert_eq!(parts, vec!["CREATE TABLE a (\n x INT\n)", "CREATE INDEX i ON a (x)"]);
    }
}
