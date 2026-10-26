//! Building blocks every domain service shares: idempotent commands, lookups and events.

use limina_core::{LiminaError, Result, Timestamp, new_uuid, now};
use limina_store::models::{Challenge, CommandReceipt, Event};
use limina_store::{Db, Tx, args};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// The outcome of starting a command: a stored result to replay, or a fresh write transaction.
pub enum Begun<T> {
    Replay(T),
    Fresh(Command),
}

/// One idempotent command. Every write the API exposes runs as a command keyed by a
/// client-supplied command ID: running it again with the same ID returns the stored result
/// instead of writing twice.
pub struct Command {
    db: Db,
    tx: Tx,
    command_id: String,
    command_type: String,
    actor: String,
}

impl Command {
    pub async fn begin<T: DeserializeOwned>(
        db: &Db,
        command_id: &str,
        command_type: &str,
        actor: &str,
    ) -> Result<Begun<T>> {
        if command_id.trim().is_empty() {
            return Err(LiminaError::invariant("A command ID is required for idempotency."));
        }
        require_text("actor", actor)?;
        let mut tx = db.begin_write().await?;
        if let Some(result) = replay(&mut tx, command_id, command_type, actor).await? {
            tx.rollback().await?;
            return Ok(Begun::Replay(result));
        }
        Ok(Begun::Fresh(Self {
            db: db.clone(),
            tx,
            command_id: command_id.to_owned(),
            command_type: command_type.to_owned(),
            actor: actor.to_owned(),
        }))
    }

    pub fn tx(&mut self) -> &mut Tx {
        &mut self.tx
    }

    pub fn command_id(&self) -> &str {
        &self.command_id
    }

    /// Stores the result as the command's receipt and commits.
    pub async fn finish<T: Serialize + DeserializeOwned>(mut self, result: T) -> Result<T> {
        let stored = serde_json::to_value(&result).map_err(limina_core::internal)?;
        let inserted = self
            .tx
            .execute(
                "INSERT INTO command_receipts (command_id, command_type, actor, result, created_at) \
                 VALUES ($1, $2, $3, $4, $5)",
                args![&self.command_id, &self.command_type, &self.actor, stored, now()],
            )
            .await;
        let committed = match inserted {
            Ok(_) => self.tx.commit().await.map_err(LiminaError::from),
            Err(error) => Err(LiminaError::from(error)),
        };
        match committed {
            Ok(()) => Ok(result),
            Err(error) => {
                resolve_race(&self.db, error, &self.command_id, &self.command_type, &self.actor)
                    .await
            }
        }
    }

    /// Rolls back after a failed operation. A constraint violation usually means a concurrent
    /// retry of the same command won the race; its stored result is returned in that case.
    pub async fn abort<T: DeserializeOwned>(self, error: LiminaError) -> Result<T> {
        let Self { db, tx, command_id, command_type, actor } = self;
        let _ = tx.rollback().await;
        resolve_race(&db, error, &command_id, &command_type, &actor).await
    }
}

async fn replay<T: DeserializeOwned>(
    tx: &mut Tx,
    command_id: &str,
    command_type: &str,
    actor: &str,
) -> Result<Option<T>> {
    let receipt: Option<CommandReceipt> = tx
        .fetch_optional("SELECT * FROM command_receipts WHERE command_id = $1", args![command_id])
        .await?;
    let Some(receipt) = receipt else {
        return Ok(None);
    };
    if receipt.actor != actor {
        return Err(LiminaError::conflict(format!(
            "Command ID '{command_id}' belongs to another actor."
        ))
        .with("command_id", command_id));
    }
    if receipt.command_type != command_type {
        return Err(LiminaError::conflict(format!(
            "Command ID '{command_id}' was already used for {}.",
            receipt.command_type
        ))
        .with("command_id", command_id)
        .with("original_command_type", receipt.command_type));
    }
    serde_json::from_value(receipt.result.0).map(Some).map_err(limina_core::internal)
}

async fn resolve_race<T: DeserializeOwned>(
    db: &Db,
    error: LiminaError,
    command_id: &str,
    command_type: &str,
    actor: &str,
) -> Result<T> {
    if !error.is_constraint_violation() {
        return Err(error);
    }
    let mut tx = db.begin().await?;
    let receipt: Option<CommandReceipt> = tx
        .fetch_optional("SELECT * FROM command_receipts WHERE command_id = $1", args![command_id])
        .await?;
    tx.commit().await?;
    match receipt {
        Some(receipt) if receipt.command_type == command_type && receipt.actor == actor => {
            serde_json::from_value(receipt.result.0).map_err(limina_core::internal)
        }
        _ => Err(LiminaError::conflict("The command conflicted with another concurrent write.")
            .with("command_id", command_id)),
    }
}

/// Runs a block as an idempotent command and evaluates to `Result<T>`:
///
/// ```ignore
/// command!(&self.db, command_id, "hypothesis.create", actor, |tx| {
///     let challenge = find_challenge(tx, slug).await?;
///     Ok(HypothesisView { .. })
/// })
/// ```
#[macro_export]
macro_rules! command {
    ($db:expr, $command_id:expr, $command_type:expr, $actor:expr, |$tx:ident| $body:block) => {{
        match $crate::common::Command::begin($db, $command_id, $command_type, $actor).await {
            Err(error) => Err(error),
            Ok($crate::common::Begun::Replay(result)) => Ok(result),
            Ok($crate::common::Begun::Fresh(mut command)) => {
                let outcome = {
                    let $tx: &mut ::limina_store::Tx = command.tx();
                    async { $body }.await
                };
                match outcome {
                    Ok(result) => command.finish(result).await,
                    Err(error) => command.abort(error).await,
                }
            }
        }
    }};
}

pub fn require_text(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        let mut chars = label.chars();
        let capitalized = chars
            .next()
            .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
            .unwrap_or_default();
        return Err(
            LiminaError::invariant(format!("{capitalized} cannot be empty.")).with("field", label)
        );
    }
    Ok(())
}

/// The project (challenge) with this slug, or a not-found error.
pub async fn find_challenge(tx: &mut Tx, slug: &str) -> Result<Challenge> {
    let challenge: Option<Challenge> = tx
        .fetch_optional("SELECT * FROM challenges WHERE slug = $1", args![slug.to_lowercase()])
        .await?;
    challenge.ok_or_else(|| {
        LiminaError::not_found(format!("Challenge '{slug}' does not exist."))
            .with("challenge", slug)
    })
}

/// Like [`find_challenge`], locking the row for the rest of the transaction on PostgreSQL.
pub async fn lock_challenge(tx: &mut Tx, slug: &str) -> Result<Challenge> {
    let sql = format!("SELECT * FROM challenges WHERE slug = $1{}", tx.for_update());
    let challenge: Option<Challenge> = tx.fetch_optional(&sql, args![slug.to_lowercase()]).await?;
    challenge.ok_or_else(|| {
        LiminaError::not_found(format!("Challenge '{slug}' does not exist."))
            .with("challenge", slug)
    })
}

/// Appends an event to a project's log and returns it with its sequence number.
pub async fn record_event(
    tx: &mut Tx,
    challenge_id: &str,
    event_type: &str,
    actor: &str,
    command_id: &str,
    payload: Value,
    artifact_id: Option<&str>,
) -> Result<Event> {
    record_event_at(tx, challenge_id, event_type, actor, command_id, payload, artifact_id, now())
        .await
}

#[allow(clippy::too_many_arguments)]
pub async fn record_event_at(
    tx: &mut Tx,
    challenge_id: &str,
    event_type: &str,
    actor: &str,
    command_id: &str,
    payload: Value,
    artifact_id: Option<&str>,
    created_at: Timestamp,
) -> Result<Event> {
    Ok(tx
        .fetch_one(
            "INSERT INTO events (id, challenge_id, event_type, actor, artifact_id, payload, \
             command_id, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING *",
            args![
                new_uuid(),
                challenge_id,
                event_type,
                actor,
                artifact_id,
                payload,
                command_id,
                created_at
            ],
        )
        .await?)
}

#[cfg(test)]
pub(crate) mod testing {
    use limina_store::Db;

    /// A fresh SQLite database in a temporary directory.
    pub async fn sqlite() -> (Db, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::connect(&format!("sqlite:///{}", dir.path().join("runtime.db").display()))
            .await
            .unwrap();
        db.initialize().await.unwrap();
        (db, dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Written {
        value: i64,
    }

    async fn write(db: &Db, command_id: &str, command_type: &str, actor: &str) -> Result<Written> {
        command!(db, command_id, command_type, actor, |tx| {
            let next: i64 =
                tx.scalar("SELECT CAST(COUNT(*) AS BIGINT) FROM command_receipts", args![]).await?;
            Ok(Written { value: next })
        })
    }

    #[tokio::test]
    async fn commands_replay_their_first_result() {
        let (db, _dir) = testing::sqlite().await;
        let first = write(&db, "cmd-1", "test.write", "Ada").await.unwrap();
        let again = write(&db, "cmd-1", "test.write", "Ada").await.unwrap();
        assert_eq!(first, again);
        assert_eq!(write(&db, "cmd-2", "test.write", "Ada").await.unwrap().value, 1);

        let other_actor = write(&db, "cmd-1", "test.write", "Grace").await.unwrap_err();
        assert_eq!(other_actor.code(), "version_conflict");
        let other_type = write(&db, "cmd-1", "test.other", "Ada").await.unwrap_err();
        assert_eq!(other_type.details["original_command_type"], "test.write");
        assert_eq!(
            write(&db, " ", "test.write", "Ada").await.unwrap_err().code(),
            "invariant_violation"
        );
    }

    #[tokio::test]
    async fn failed_commands_leave_no_receipt() {
        let (db, _dir) = testing::sqlite().await;
        let failed: Result<Written> = command!(&db, "cmd-x", "test.fail", "Ada", |tx| {
            tx.execute("DELETE FROM command_receipts", args![]).await?;
            Err(LiminaError::invariant("nope"))
        });
        assert_eq!(failed.unwrap_err().message, "nope");
        assert_eq!(write(&db, "cmd-x", "test.write", "Ada").await.unwrap().value, 0);
    }
}
