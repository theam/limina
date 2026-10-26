//! The hidden `db` commands: schema upgrades and the current revision.

use limina_core::LiminaError;
use limina_store::db::Backend;
use limina_store::schema::{HEAD, POSTGRES_MIGRATIONS};
use limina_store::{Db, args};
use serde_json::json;

use super::{Context, Outcome, database_url};
use crate::cli::DbCommand;
use crate::client::local::connect;
use crate::output;

pub async fn database(context: &Context, command: &DbCommand) -> Outcome {
    let url = database_url(&context.database);
    let db = connect(&url).await?;
    let result = match command {
        DbCommand::Upgrade => upgrade(context, &db, &url).await,
        DbCommand::Current => current(&db, &url).await,
    };
    db.close().await;
    result
}

/// Brings the schema to the newest revision (SQLite is created or completed in place).
async fn upgrade(context: &Context, db: &Db, url: &str) -> Outcome {
    db.upgrade().await?;
    let result = json!({"database": hide_password(url), "revision": "head"});
    context.out.emit(&result, |value| {
        format!(
            "Database  {}\nRevision  {}",
            output::field(value, "database"),
            output::field(value, "revision")
        )
    });
    Ok(())
}

/// Prints the revision the way `alembic current --verbose` did.
async fn current(db: &Db, url: &str) -> Outcome {
    let revision = match db.backend() {
        // SQLite has no revision table: a database with the schema is at the head.
        Backend::Sqlite => {
            let mut tx = db.begin().await.map_err(LiminaError::from)?;
            let tables: i64 = tx
                .scalar(
                    "SELECT CAST(COUNT(*) AS BIGINT) FROM sqlite_master \
                     WHERE type = 'table' AND name = $1",
                    args!["challenges"],
                )
                .await
                .map_err(LiminaError::from)?;
            tx.commit().await.map_err(LiminaError::from)?;
            (tables > 0).then(|| HEAD.to_owned())
        }
        Backend::Postgres => db.revision().await?,
    };
    let mut lines = vec![format!("Current revision(s) for {}:", hide_password(url))];
    if let Some(revision) = revision {
        let migration = POSTGRES_MIGRATIONS.iter().find(|m| m.revision == revision);
        let head = if revision == HEAD { " (head)" } else { "" };
        lines.push(format!("Rev: {revision}{head}"));
        if let Some(migration) = migration {
            lines.push(format!("Parent: {}", migration.previous.unwrap_or("<base>")));
            lines.push(String::new());
            lines.push(format!("    {}", migration.description));
        }
    }
    output::line(&lines.join("\n"));
    Ok(())
}

/// The URL with its password replaced by `***`, as Alembic printed it.
pub fn hide_password(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(authority_end);
    let Some((userinfo, host)) = authority.rsplit_once('@') else {
        return url.to_owned();
    };
    match userinfo.split_once(':') {
        Some((user, _password)) => format!("{scheme}://{user}:***@{host}{tail}"),
        None => url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_never_reach_the_output() {
        assert_eq!(
            hide_password("postgresql+psycopg://limina:s3cr@t@db:5432/limina?sslmode=require"),
            "postgresql+psycopg://limina:***@db:5432/limina?sslmode=require"
        );
        assert_eq!(hide_password("postgres://limina@db/limina"), "postgres://limina@db/limina");
        assert_eq!(
            hide_password("sqlite:////var/lib/limina/runtime.db"),
            "sqlite:////var/lib/limina/runtime.db"
        );
    }
}
