//! The transactional command service for the Limina challenge runtime.
//!
//! The service owns every research-graph invariant. Transports (CLI, HTTP, MCP, the supervisor)
//! call these commands without reimplementing the rules. Project lifecycle and resources live in
//! [`crate::project`], the H → E → F commands in [`crate::research`]; all of them are methods of
//! [`ChallengeService`].

pub(crate) mod internal;
mod views;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use limina_core::{LiminaError, Result, Timestamp, iso, iso_opt, new_uuid, now};
use limina_store::models::{Artifact, CoordinatorState, Event, InboxMessage, WorkLease};
use limina_store::{Arg, Db, SecretCipher, Tx, args};
use serde_json::{Map, Value, json};

use crate::command;
use crate::common::{find_challenge, record_event};
pub use internal::RESOURCE_VALUE_LIMIT;
use internal::{
    MESSAGE_LIMIT, acquire_lease, placeholders, require_text, strip, validate_lease_ttl,
};
pub use views::{
    ArtifactDetail, ArtifactView, ChallengeView, CoordinatorRelease, CoordinatorView, EventView,
    ExperimentClaim, LeaseGrant, LeaseView, MessageView, ObservationView, ResourceView, StatusView,
};

/// The coordinator states a checkpoint may record.
pub const RUNTIME_STATUSES: [&str; 7] =
    ["CREATED", "RUNNING", "WAITING", "PAUSED", "STOPPED", "COMPLETE", "FAILED"];
/// The kinds of guidance a person can send.
pub const MESSAGE_KINDS: [&str; 6] =
    ["STEER", "INTERRUPT", "ANSWER", "APPROVAL", "COMMENT", "BLOCKER"];
/// The most events one page returns.
pub const MAX_EVENT_PAGE: i64 = 1000;

/// The project's research service: one database, and the cipher for project secrets.
#[derive(Debug, Clone)]
pub struct ChallengeService {
    pub(crate) db: Db,
    pub(crate) secret_cipher: Option<Arc<SecretCipher>>,
}

/// A coordinator checkpoint: the durable working state after a turn.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Checkpoint {
    pub current_objective: String,
    pub next_step: String,
    /// Empty means `None`.
    pub blocker: String,
    /// One of [`RUNTIME_STATUSES`], case-insensitive.
    pub status: String,
    pub worker_id: Option<String>,
    pub continuation_id: Option<String>,
    pub inbox_cursor: i32,
    /// The coordinator version this checkpoint was based on.
    pub expected_version: i32,
    /// Inbox messages the turn consumed; they are acknowledged with the checkpoint.
    pub acknowledge_message_ids: Vec<String>,
    pub wake_at: Option<Timestamp>,
}

impl ChallengeService {
    pub fn new(db: Db, secret_cipher: Option<Arc<SecretCipher>>) -> Self {
        Self { db, secret_cipher }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn secret_cipher(&self) -> Option<&SecretCipher> {
        self.secret_cipher.as_deref()
    }

    pub async fn get_challenge(&self, slug: &str) -> Result<ChallengeView> {
        let mut tx = self.db.begin().await?;
        let challenge = find_challenge(&mut tx, slug).await?;
        let coordinator = find_coordinator(&mut tx, &challenge.id).await?;
        tx.commit().await?;
        Ok(ChallengeView::new(&challenge, coordinator.as_ref()))
    }

    pub async fn status(&self, slug: &str) -> Result<StatusView> {
        let mut tx = self.db.begin().await?;
        let challenge = find_challenge(&mut tx, slug).await?;
        let coordinator = find_coordinator(&mut tx, &challenge.id).await?;
        let grouped: Vec<(String, String, i64)> = tx
            .fetch_all(
                "SELECT kind, status, CAST(COUNT(*) AS BIGINT) FROM artifacts \
                 WHERE challenge_id = $1 GROUP BY kind, status ORDER BY kind, status",
                args![&challenge.id],
            )
            .await?;
        let mut counts: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
        for (kind, status, count) in grouped {
            counts.entry(kind).or_default().insert(status, count);
        }
        let running: Vec<Artifact> = tx
            .fetch_all(
                "SELECT * FROM artifacts WHERE challenge_id = $1 AND kind = 'E' \
                 AND status = 'RUNNING' ORDER BY artifact_id",
                args![&challenge.id],
            )
            .await?;
        let pending_inbox: i64 = tx
            .scalar(
                "SELECT CAST(COUNT(*) AS BIGINT) FROM inbox_messages \
                 WHERE challenge_id = $1 AND status = 'PENDING'",
                args![&challenge.id],
            )
            .await?;
        let last_event: Option<i64> = tx
            .scalar(
                "SELECT CAST(MAX(sequence) AS BIGINT) FROM events WHERE challenge_id = $1",
                args![&challenge.id],
            )
            .await?;
        tx.commit().await?;
        Ok(StatusView {
            challenge: ChallengeView::new(&challenge, coordinator.as_ref()),
            counts,
            running_experiments: running.iter().map(ArtifactView::from).collect(),
            pending_inbox,
            last_event_sequence: last_event.unwrap_or(0),
        })
    }

    /// Takes the project's coordinator lease, so only one worker drives the project at a time.
    pub async fn claim_coordinator(
        &self,
        slug: &str,
        ttl_seconds: i64,
        actor: &str,
        command_id: &str,
    ) -> Result<LeaseView> {
        validate_lease_ttl(ttl_seconds)?;
        command!(&self.db, command_id, "coordinator.claim", actor, |tx| {
            let challenge = find_challenge(tx, slug).await?;
            let lease = acquire_lease(tx, &challenge.id, "coordinator", actor, ttl_seconds).await?;
            let expires_at = iso(&lease.expires_at);
            record_event(
                tx,
                &challenge.id,
                "coordinator.claimed",
                actor,
                command_id,
                json!({"expires_at": expires_at}),
                None,
            )
            .await?;
            Ok(LeaseView::from(&lease))
        })
    }

    pub async fn release_coordinator(
        &self,
        slug: &str,
        actor: &str,
        command_id: &str,
    ) -> Result<CoordinatorRelease> {
        command!(&self.db, command_id, "coordinator.release", actor, |tx| {
            let challenge = find_challenge(tx, slug).await?;
            let deleted = tx
                .execute(
                    "DELETE FROM work_leases WHERE challenge_id = $1 AND scope = 'coordinator' \
                     AND owner = $2",
                    args![&challenge.id, actor],
                )
                .await?;
            if deleted != 1 {
                let existing = internal::find_lease(tx, &challenge.id, "coordinator").await?;
                return Err(LiminaError::lease_conflict(
                    "The coordinator lease is not owned by this worker.",
                )
                .with("scope", "coordinator")
                .with("owner", existing.map(|lease| lease.owner))
                .with("actor", actor));
            }
            record_event(
                tx,
                &challenge.id,
                "coordinator.released",
                actor,
                command_id,
                json!({}),
                None,
            )
            .await?;
            Ok(CoordinatorRelease {
                scope: "coordinator".to_owned(),
                owner: actor.to_owned(),
                released: true,
            })
        })
    }

    /// Records the coordinator's working state if it is still at `expected_version`, and
    /// acknowledges the guidance the turn consumed in the same transaction.
    pub async fn checkpoint_coordinator(
        &self,
        slug: &str,
        checkpoint: Checkpoint,
        actor: &str,
        command_id: &str,
    ) -> Result<CoordinatorView> {
        require_text("current objective", &checkpoint.current_objective)?;
        require_text("next step", &checkpoint.next_step)?;
        let status = checkpoint.status.to_uppercase();
        if !RUNTIME_STATUSES.contains(&status.as_str()) {
            return Err(LiminaError::invariant(
                "Runtime status must be CREATED, RUNNING, WAITING, PAUSED, STOPPED, COMPLETE, \
                 or FAILED.",
            ));
        }
        let message_ids: BTreeSet<&str> =
            checkpoint.acknowledge_message_ids.iter().map(String::as_str).collect();
        command!(&self.db, command_id, "coordinator.checkpoint", actor, |tx| {
            let challenge = find_challenge(tx, slug).await?;
            if !message_ids.is_empty() {
                let mut values: Vec<Arg> = args![&challenge.id];
                values.extend(message_ids.iter().map(|id| Arg::from(*id)));
                let found: Vec<(String,)> = tx
                    .fetch_all(
                        &format!(
                            "SELECT id FROM inbox_messages WHERE challenge_id = $1 AND id IN ({})",
                            placeholders(2, message_ids.len())
                        ),
                        values,
                    )
                    .await?;
                if found.len() != message_ids.len() {
                    let found: BTreeSet<&str> = found.iter().map(|(id,)| id.as_str()).collect();
                    let missing: Vec<&str> = message_ids.difference(&found).copied().collect();
                    return Err(LiminaError::not_found(
                        "One or more inbox messages do not exist in this challenge.",
                    )
                    .with("message_ids", missing));
                }
            }
            let changed_at = now();
            let blocker = match strip(&checkpoint.blocker) {
                "" => "None",
                blocker => blocker,
            };
            let coordinator: Option<CoordinatorState> = tx
                .fetch_optional(
                    "UPDATE coordinator_states SET current_objective = $1, next_step = $2, \
                     blocker = $3, status = $4, worker_id = $5, continuation_id = $6, \
                     inbox_cursor = $7, heartbeat_at = $8, wake_at = $9, updated_at = $8, \
                     version = version + 1 WHERE challenge_id = $10 AND version = $11 RETURNING *",
                    args![
                        strip(&checkpoint.current_objective),
                        strip(&checkpoint.next_step),
                        blocker,
                        status.as_str(),
                        checkpoint.worker_id.as_deref(),
                        checkpoint.continuation_id.as_deref(),
                        checkpoint.inbox_cursor,
                        changed_at,
                        checkpoint.wake_at,
                        &challenge.id,
                        checkpoint.expected_version
                    ],
                )
                .await?;
            let Some(coordinator) = coordinator else {
                let current = find_coordinator(tx, &challenge.id).await?.map(|row| row.version);
                let shown = current.map_or_else(|| "?".to_owned(), |version| version.to_string());
                return Err(LiminaError::conflict(format!(
                    "Coordinator state changed from v{} to v{shown}.",
                    checkpoint.expected_version
                ))
                .with("expected_version", checkpoint.expected_version)
                .with("current_version", current));
            };
            if !message_ids.is_empty() {
                let mut values: Vec<Arg> = args![now(), &challenge.id];
                values.extend(message_ids.iter().map(|id| Arg::from(*id)));
                tx.execute(
                    &format!(
                        "UPDATE inbox_messages SET status = 'ACKNOWLEDGED', acknowledged_at = $1 \
                         WHERE challenge_id = $2 AND id IN ({})",
                        placeholders(3, message_ids.len())
                    ),
                    values,
                )
                .await?;
            }
            record_event(
                tx,
                &challenge.id,
                "coordinator.checkpointed",
                actor,
                command_id,
                json!({
                    "status": status,
                    "current_objective": coordinator.current_objective,
                    "next_step": coordinator.next_step,
                    "blocker": coordinator.blocker,
                    "version": coordinator.version,
                    "worker_id": checkpoint.worker_id,
                    "continuation_id": checkpoint.continuation_id,
                    "inbox_cursor": checkpoint.inbox_cursor,
                    "messages_acknowledged": message_ids.len(),
                    "wake_at": iso_opt(checkpoint.wake_at.as_ref()),
                }),
                None,
            )
            .await?;
            Ok(CoordinatorView::from(&coordinator))
        })
    }

    /// Queues durable human guidance. `origin` marks guidance generated by a product action (for
    /// example `resource` when a variable or secret changes) so clients can present it as an
    /// event rather than a typed note.
    pub async fn send_message(
        &self,
        slug: &str,
        kind: &str,
        body: &str,
        origin: Option<&str>,
        actor: &str,
        command_id: &str,
    ) -> Result<MessageView> {
        let kind = kind.to_uppercase();
        if !MESSAGE_KINDS.contains(&kind.as_str()) {
            return Err(LiminaError::invariant(
                "Message kind must be STEER, INTERRUPT, ANSWER, APPROVAL, COMMENT, or BLOCKER.",
            ));
        }
        require_text("message", body)?;
        if body.chars().count() > MESSAGE_LIMIT {
            return Err(LiminaError::invariant("Message must be at most 32768 characters."));
        }
        command!(&self.db, command_id, "inbox.send", actor, |tx| {
            let challenge = find_challenge(tx, slug).await?;
            let message: InboxMessage = tx
                .fetch_one(
                    "INSERT INTO inbox_messages (id, challenge_id, kind, body, actor, status, \
                     command_id, created_at, acknowledged_at) \
                     VALUES ($1, $2, $3, $4, $5, 'PENDING', $6, $7, NULL) RETURNING *",
                    args![
                        new_uuid(),
                        &challenge.id,
                        kind.as_str(),
                        strip(body),
                        actor,
                        command_id,
                        now()
                    ],
                )
                .await?;
            let mut payload = json!({
                "message_id": message.id,
                "kind": kind,
                "sequence": message.sequence,
                "body": message.body,
            });
            if let (Some(origin), Some(object)) =
                (origin.filter(|origin| !origin.is_empty()), payload.as_object_mut())
            {
                object.insert("origin".to_owned(), Value::from(origin));
            }
            record_event(tx, &challenge.id, "inbox.message_sent", actor, command_id, payload, None)
                .await?;
            Ok(MessageView::from(&message))
        })
    }

    /// Guidance text by message ID, for events recorded before bodies were part of the payload.
    pub async fn message_bodies<I, S>(
        &self,
        slug: &str,
        message_ids: I,
    ) -> Result<BTreeMap<String, String>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let ids: BTreeSet<String> =
            message_ids.into_iter().map(|id| id.as_ref().to_owned()).collect();
        if ids.is_empty() {
            return Ok(BTreeMap::new());
        }
        let mut tx = self.db.begin().await?;
        let challenge = find_challenge(&mut tx, slug).await?;
        let mut values: Vec<Arg> = args![&challenge.id];
        values.extend(ids.iter().map(Arg::from));
        let rows: Vec<(String, String)> = tx
            .fetch_all(
                &format!(
                    "SELECT id, body FROM inbox_messages WHERE challenge_id = $1 AND id IN ({})",
                    placeholders(2, ids.len())
                ),
                values,
            )
            .await?;
        tx.commit().await?;
        Ok(rows.into_iter().collect())
    }

    /// Guidance after inbox sequence `after`, oldest first.
    pub async fn inbox(
        &self,
        slug: &str,
        after: i64,
        pending_only: bool,
    ) -> Result<Vec<MessageView>> {
        let mut tx = self.db.begin().await?;
        let challenge = find_challenge(&mut tx, slug).await?;
        let sql = if pending_only {
            "SELECT * FROM inbox_messages WHERE challenge_id = $1 AND sequence > $2 \
             AND status = 'PENDING' ORDER BY sequence"
        } else {
            "SELECT * FROM inbox_messages WHERE challenge_id = $1 AND sequence > $2 \
             ORDER BY sequence"
        };
        let messages: Vec<InboxMessage> = tx.fetch_all(sql, args![&challenge.id, after]).await?;
        tx.commit().await?;
        Ok(messages.iter().map(MessageView::from).collect())
    }

    pub async fn acknowledge_message(
        &self,
        slug: &str,
        message_id: &str,
        actor: &str,
        command_id: &str,
    ) -> Result<MessageView> {
        command!(&self.db, command_id, "inbox.acknowledge", actor, |tx| {
            let challenge = find_challenge(tx, slug).await?;
            let message: InboxMessage = tx
                .fetch_optional(
                    "UPDATE inbox_messages SET status = 'ACKNOWLEDGED', acknowledged_at = $1 \
                     WHERE challenge_id = $2 AND id = $3 RETURNING *",
                    args![now(), &challenge.id, message_id],
                )
                .await?
                .ok_or_else(|| {
                    LiminaError::not_found(format!("Inbox message '{message_id}' does not exist."))
                })?;
            record_event(
                tx,
                &challenge.id,
                "inbox.message_acknowledged",
                actor,
                command_id,
                json!({"message_id": message.id, "sequence": message.sequence}),
                None,
            )
            .await?;
            Ok(MessageView::from(&message))
        })
    }

    /// Latest event of one type, independent of transcript pagination.
    pub async fn latest_event(&self, slug: &str, event_type: &str) -> Result<Option<EventView>> {
        let mut tx = self.db.begin().await?;
        let challenge = find_challenge(&mut tx, slug).await?;
        let event: Option<Event> = tx.fetch_optional(
            "SELECT * FROM events WHERE challenge_id = $1 AND event_type = $2 ORDER BY sequence DESC LIMIT 1",
            args![&challenge.id, event_type],
        ).await?;
        tx.commit().await?;
        Ok(event.as_ref().map(EventView::from))
    }

    /// Events after sequence `after`, oldest first, at most `limit` (clamped to 1–1000).
    pub async fn events(&self, slug: &str, after: i64, limit: i64) -> Result<Vec<EventView>> {
        let mut tx = self.db.begin().await?;
        let challenge = find_challenge(&mut tx, slug).await?;
        let events: Vec<Event> = tx
            .fetch_all(
                "SELECT * FROM events WHERE challenge_id = $1 AND sequence > $2 \
                 ORDER BY sequence LIMIT $3",
                args![&challenge.id, after, page_limit(limit)],
            )
            .await?;
        tx.commit().await?;
        Ok(events.iter().map(EventView::from).collect())
    }

    /// The newest events below `before` (and above `after`) in ascending order, and whether older
    /// events exist. `exclude_types` hides event types from both.
    pub async fn events_before(
        &self,
        slug: &str,
        before: i64,
        after: i64,
        limit: i64,
        exclude_types: &[&str],
    ) -> Result<(Vec<EventView>, bool)> {
        let mut tx = self.db.begin().await?;
        let challenge = find_challenge(&mut tx, slug).await?;
        let (filter, mut values) = event_filters(&challenge.id, exclude_types);
        let next = values.len() + 1;
        values.extend(args![before, after, page_limit(limit)]);
        let mut events: Vec<Event> = tx
            .fetch_all(
                &format!(
                    "SELECT * FROM events WHERE {filter} AND sequence < ${next} \
                     AND sequence > ${} ORDER BY sequence DESC LIMIT ${}",
                    next + 1,
                    next + 2
                ),
                values,
            )
            .await?;
        events.reverse();
        let has_earlier = match events.first() {
            Some(oldest) => {
                has_event_before(&mut tx, &challenge.id, exclude_types, i64::from(oldest.sequence))
                    .await?
            }
            None => false,
        };
        tx.commit().await?;
        Ok((events.iter().map(EventView::from).collect(), has_earlier))
    }

    /// Whether any event (not of `exclude_types`) precedes `sequence`.
    pub async fn has_events_before(
        &self,
        slug: &str,
        sequence: i64,
        exclude_types: &[&str],
    ) -> Result<bool> {
        let mut tx = self.db.begin().await?;
        let challenge = find_challenge(&mut tx, slug).await?;
        let earlier = has_event_before(&mut tx, &challenge.id, exclude_types, sequence).await?;
        tx.commit().await?;
        Ok(earlier)
    }

    /// The newest events in chronological order, without a forward scan of the whole history.
    pub async fn recent_events(&self, slug: &str, limit: i64) -> Result<Vec<EventView>> {
        let mut tx = self.db.begin().await?;
        let challenge = find_challenge(&mut tx, slug).await?;
        let mut events: Vec<Event> = tx
            .fetch_all(
                "SELECT * FROM events WHERE challenge_id = $1 ORDER BY sequence DESC LIMIT $2",
                args![&challenge.id, page_limit(limit)],
            )
            .await?;
        tx.commit().await?;
        events.reverse();
        Ok(events.iter().map(EventView::from).collect())
    }

    /// Appends an internal `runtime.*` event (engine transcript, usage, resource refreshes). The
    /// event type is also the command type.
    pub async fn record_runtime_event(
        &self,
        slug: &str,
        event_type: &str,
        payload: Map<String, Value>,
        actor: &str,
        command_id: &str,
    ) -> Result<EventView> {
        if !event_type.starts_with("runtime.") {
            return Err(LiminaError::invariant(
                "Internal runtime event types must start with 'runtime.'.",
            )
            .with("event_type", event_type));
        }
        command!(&self.db, command_id, event_type, actor, |tx| {
            let challenge = find_challenge(tx, slug).await?;
            let event = record_event(
                tx,
                &challenge.id,
                event_type,
                actor,
                command_id,
                Value::Object(payload),
                None,
            )
            .await?;
            Ok(EventView::from(&event))
        })
    }
}

pub(crate) async fn find_coordinator(
    tx: &mut Tx,
    challenge_id: &str,
) -> Result<Option<CoordinatorState>> {
    Ok(tx
        .fetch_optional(
            "SELECT * FROM coordinator_states WHERE challenge_id = $1",
            args![challenge_id],
        )
        .await?)
}

/// A page size within 1–1000.
fn page_limit(limit: i64) -> i64 {
    limit.clamp(1, MAX_EVENT_PAGE)
}

/// `challenge_id = $1 [AND event_type NOT IN (…)]` and its values.
fn event_filters(challenge_id: &str, exclude_types: &[&str]) -> (String, Vec<Arg>) {
    let mut filter = "challenge_id = $1".to_owned();
    let mut values = args![challenge_id];
    if !exclude_types.is_empty() {
        filter.push_str(&format!(
            " AND event_type NOT IN ({})",
            placeholders(2, exclude_types.len())
        ));
        values.extend(exclude_types.iter().map(|value| Arg::from(*value)));
    }
    (filter, values)
}

async fn has_event_before(
    tx: &mut Tx,
    challenge_id: &str,
    exclude_types: &[&str],
    sequence: i64,
) -> Result<bool> {
    let (filter, mut values) = event_filters(challenge_id, exclude_types);
    let next = values.len() + 1;
    values.push(Arg::from(sequence));
    let earlier: Option<i32> = tx
        .scalar_optional(
            &format!("SELECT sequence FROM events WHERE {filter} AND sequence < ${next} LIMIT 1"),
            values,
        )
        .await?;
    Ok(earlier.is_some())
}

/// The lease on an experiment, when one is held and unexpired.
pub(crate) fn active_lease(lease: Option<WorkLease>) -> Option<WorkLease> {
    lease.filter(|lease| lease.expires_at > now())
}
