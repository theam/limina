//! Schema creation and migrations, compatible with databases created by the Python runtime.
//!
//! - SQLite creates any missing table and index on startup, like SQLAlchemy's `create_all`, and
//!   adds the columns introduced after a table was first created ([`SQLITE_ADDED_COLUMNS`]).
//! - PostgreSQL keeps Alembic's `alembic_version` table and revision IDs, so a database migrated
//!   by the Python runtime continues from where it is. `limina db upgrade` applies the missing
//!   revisions in one transaction under an advisory lock.

use limina_core::{LiminaError, Result};

use crate::db::{Db, DbError, Tx};

const SQLITE_SCHEMA: &str = include_str!("../schema/sqlite.sql");

/// One PostgreSQL schema revision, identified by its Alembic revision ID.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub revision: &'static str,
    pub previous: Option<&'static str>,
    pub description: &'static str,
    pub sql: &'static str,
}

pub const POSTGRES_MIGRATIONS: &[Migration] = &[
    Migration {
        revision: "474c29565487",
        previous: None,
        description: "Initial challenge runtime.",
        sql: include_str!("../schema/postgres/0001_474c29565487.sql"),
    },
    Migration {
        revision: "9a62d4f771c1",
        previous: Some("474c29565487"),
        description: "Resource variables and secrets.",
        sql: include_str!("../schema/postgres/0002_9a62d4f771c1.sql"),
    },
    Migration {
        revision: "c0d3c1a2b7e9",
        previous: Some("9a62d4f771c1"),
        description: "Selectable runtime engines.",
        sql: include_str!("../schema/postgres/0003_c0d3c1a2b7e9.sql"),
    },
    Migration {
        revision: "7e4a19b8d2c6",
        previous: Some("c0d3c1a2b7e9"),
        description: "UI-ready control plane.",
        sql: include_str!("../schema/postgres/0004_7e4a19b8d2c6.sql"),
    },
    Migration {
        revision: "1f6a2c8d9e10",
        previous: Some("7e4a19b8d2c6"),
        description: "Per-turn token detail and cost provenance.",
        sql: include_str!("../schema/postgres/0005_1f6a2c8d9e10.sql"),
    },
    Migration {
        revision: "5b8e1d3f7a20",
        previous: Some("1f6a2c8d9e10"),
        description: "Per-project engine accounts.",
        sql: include_str!("../schema/postgres/0006_5b8e1d3f7a20.sql"),
    },
    Migration {
        revision: "8d3f2b6c1a47",
        previous: Some("5b8e1d3f7a20"),
        description: "Private projects shared by email.",
        sql: include_str!("../schema/postgres/0007_8d3f2b6c1a47.sql"),
    },
    Migration {
        revision: "8c4f2a6e1d93",
        previous: Some("8d3f2b6c1a47"),
        description: "Personal MCP connections and project permissions.",
        sql: include_str!("../schema/postgres/0008_8c4f2a6e1d93.sql"),
    },
    Migration {
        revision: "d27e6c4a9b10",
        previous: Some("8c4f2a6e1d93"),
        description: "Project model and reasoning defaults.",
        sql: include_str!("../schema/postgres/0009_d27e6c4a9b10.sql"),
    },
    Migration {
        revision: "e4c7a2d9b601",
        previous: Some("d27e6c4a9b10"),
        description: "Evidence-backed project KPIs.",
        sql: include_str!("../schema/postgres/0010_e4c7a2d9b601.sql"),
    },
];

/// The newest schema revision this build understands.
pub const HEAD: &str = "e4c7a2d9b601";

/// Columns added to an existing SQLite table after it was first created, as
/// `(table, column, definition)`. `CREATE TABLE IF NOT EXISTS` leaves an existing table alone, so
/// [`Db::initialize`] adds each one that is missing. New databases get them from the schema file.
pub const SQLITE_ADDED_COLUMNS: &[(&str, &str, &str)] = &[
    ("live_tickets", "project_admin", "BOOLEAN DEFAULT 0 NOT NULL"),
    ("challenges", "model", "VARCHAR(200)"),
    ("challenges", "reasoning_effort", "VARCHAR(64)"),
];

/// Serializes concurrent `db upgrade` runs against one PostgreSQL database.
const UPGRADE_LOCK: i64 = 0x4c49_4d49_4e41; // "LIMINA"

impl Db {
    /// Prepares the database for serving. SQLite is created or completed in place; PostgreSQL
    /// must already be at [`HEAD`] (run `limina db upgrade` first).
    pub async fn initialize(&self) -> Result<()> {
        match self {
            Db::Sqlite(_) => {
                let mut tx = self.begin_write().await?;
                tx.execute_script(SQLITE_SCHEMA).await?;
                for (table, column, definition) in SQLITE_ADDED_COLUMNS {
                    let present: i64 = tx
                        .scalar(
                            "SELECT CAST(COUNT(*) AS BIGINT) FROM pragma_table_info($1) \
                             WHERE name = $2",
                            crate::args![*table, *column],
                        )
                        .await?;
                    if present == 0 {
                        // Identifiers cannot be bound; these are constants of this module.
                        tx.execute_script(&format!(
                            "ALTER TABLE {table} ADD COLUMN {column} {definition};"
                        ))
                        .await?;
                    }
                }
                tx.commit().await?;
                Ok(())
            }
            Db::Postgres(_) => match self.revision().await? {
                Some(revision) if revision == HEAD => Ok(()),
                Some(revision) if !POSTGRES_MIGRATIONS.iter().any(|m| m.revision == revision) => {
                    Err(LiminaError::invariant(
                        "The shared database was migrated by a newer Limina release.",
                    )
                    .with("revision", revision)
                    .with("supported_head", HEAD))
                }
                Some(revision) => Err(LiminaError::invariant(
                    "The shared database is not on the current schema. Run `limina db upgrade`.",
                )
                .with("revision", revision)
                .with("head", HEAD)),
                None => Err(LiminaError::invariant(
                    "The shared database is not migrated. Run `limina --database <url> db upgrade`.",
                )),
            },
        }
    }

    /// The Alembic revision of a PostgreSQL database (`None` when unmigrated). SQLite has no
    /// revisions and always reports [`HEAD`] once initialized.
    pub async fn revision(&self) -> Result<Option<String>> {
        let mut tx = self.begin().await?;
        let revision = revision_in(&mut tx).await?;
        tx.commit().await?;
        Ok(revision)
    }

    /// Brings the schema to [`HEAD`] and returns the revisions it applied.
    pub async fn upgrade(&self) -> Result<Vec<&'static str>> {
        if let Db::Sqlite(_) = self {
            self.initialize().await?;
            return Ok(Vec::new());
        }
        let mut tx = self.begin_write().await?;
        tx.execute("SELECT pg_advisory_xact_lock($1)", crate::args![UPGRADE_LOCK]).await?;
        tx.execute(
            "CREATE TABLE IF NOT EXISTS alembic_version (\
                version_num VARCHAR(32) NOT NULL, \
                CONSTRAINT alembic_version_pkc PRIMARY KEY (version_num))",
            Vec::new(),
        )
        .await?;
        let current = revision_in(&mut tx).await?;
        let start = match current.as_deref() {
            None => 0,
            Some(revision) => {
                let position = POSTGRES_MIGRATIONS
                    .iter()
                    .position(|m| m.revision == revision)
                    .ok_or_else(|| {
                        LiminaError::invariant(
                            "The shared database was migrated by a newer Limina release.",
                        )
                        .with("revision", revision)
                    })?;
                position + 1
            }
        };
        let mut applied = Vec::new();
        for migration in &POSTGRES_MIGRATIONS[start..] {
            tracing::info!(revision = migration.revision, "applying schema migration");
            tx.execute_script(migration.sql).await?;
            match migration.previous {
                None => {
                    tx.execute(
                        "INSERT INTO alembic_version (version_num) VALUES ($1)",
                        crate::args![migration.revision],
                    )
                    .await?;
                }
                Some(previous) => {
                    tx.execute(
                        "UPDATE alembic_version SET version_num = $1 WHERE version_num = $2",
                        crate::args![migration.revision, previous],
                    )
                    .await?;
                }
            }
            applied.push(migration.revision);
        }
        tx.commit().await?;
        Ok(applied)
    }
}

async fn revision_in(tx: &mut Tx) -> Result<Option<String>, DbError> {
    match tx {
        Tx::Sqlite(_) => Ok(Some(HEAD.to_owned())),
        Tx::Postgres(_) => {
            let exists: bool = tx
                .scalar("SELECT to_regclass('public.alembic_version') IS NOT NULL", Vec::new())
                .await?;
            if !exists {
                return Ok(None);
            }
            tx.scalar_optional("SELECT version_num FROM alembic_version LIMIT 1", Vec::new()).await
        }
    }
}
