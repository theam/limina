//! Fixtures shared by the collaboration and knowledge tests.
//!
//! Every test runs on SQLite, and on PostgreSQL as well when `LIMINA_TEST_POSTGRES_URL` points at
//! a database the tests may wipe (see `limina-store/tests/backends.rs`). PostgreSQL tests in one
//! binary take turns, because each one recreates the `public` schema.
//!
//! Projects and artifacts are inserted with plain SQL, shaped like the rows the project and
//! research services write.

#![allow(dead_code, clippy::unwrap_used)]

use chrono::{TimeZone, Utc};
use limina_core::{Principal, Timestamp, new_uuid, now};
use limina_domain::collaboration::CollaborationService;
use limina_store::{Db, args};
use serde_json::Value;
use tokio::sync::{Mutex, MutexGuard};

static POSTGRES: Mutex<()> = Mutex::const_new(());

/// A migrated, empty database on one backend.
pub struct Backend {
    pub db: Db,
    pub name: &'static str,
    _dir: Option<tempfile::TempDir>,
    _turn: Option<MutexGuard<'static, ()>>,
}

impl Backend {
    pub fn service(&self) -> CollaborationService {
        CollaborationService::new(self.db.clone())
    }

    pub fn is_postgres(&self) -> bool {
        self.name == "postgres"
    }
}

/// SQLite always; PostgreSQL when `LIMINA_TEST_POSTGRES_URL` is set.
pub async fn backends() -> Vec<Backend> {
    let dir = tempfile::tempdir().unwrap();
    let sqlite = Db::connect(&format!("sqlite:///{}", dir.path().join("runtime.db").display()))
        .await
        .unwrap();
    sqlite.initialize().await.unwrap();
    let mut all = vec![Backend { db: sqlite, name: "sqlite", _dir: Some(dir), _turn: None }];
    if let Ok(url) = std::env::var("LIMINA_TEST_POSTGRES_URL") {
        let turn = POSTGRES.lock().await;
        let db = Db::connect(&url).await.unwrap();
        let mut tx = db.begin_write().await.unwrap();
        tx.execute_script("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").await.unwrap();
        tx.commit().await.unwrap();
        db.upgrade().await.unwrap();
        db.initialize().await.unwrap();
        all.push(Backend { db, name: "postgres", _dir: None, _turn: Some(turn) });
    }
    all
}

pub fn owner() -> Principal {
    Principal::new("owner", "Olivia Owner").with_email(Some("owner@example.test".to_owned()))
}

pub fn viewer() -> Principal {
    Principal::new("viewer", "Victor Viewer").with_email(Some("viewer@example.test".to_owned()))
}

pub fn editor() -> Principal {
    Principal::new("editor", "Erin Editor").with_email(Some("editor@example.test".to_owned()))
}

/// Another person who happens to share the owner's display name.
pub fn same_name() -> Principal {
    Principal::new("other-owner", "Olivia Owner").with_email(Some("other@example.test".to_owned()))
}

pub fn admin() -> Principal {
    Principal::new("instance-admin", "Alice Admin")
        .with_email(Some("admin@example.test".to_owned()))
        .admin(true)
}

/// A fixed instant: 2026-10-`day` `hour`:00 UTC, plus `micros` microseconds.
pub fn at(day: u32, hour: u32, micros: i64) -> Timestamp {
    Utc.with_ymd_and_hms(2026, 10, day, hour, 0, 0).unwrap()
        + chrono::Duration::microseconds(micros)
}

/// A project as `create_challenge` leaves it: never started, owned by `owner` when given.
/// Returns the challenge ID.
pub async fn create_project(db: &Db, slug: &str, owner: Option<&Principal>) -> String {
    let id = new_uuid();
    let created = now();
    let mut tx = db.begin_write().await.unwrap();
    tx.execute(
        "INSERT INTO challenges (id, slug, name, objective, context, success_criteria, \
         runtime_engine, status, version, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, '', $5, 'codex', 'ACTIVE', 1, $6, $6)",
        args![
            &id,
            slug,
            "UI ready",
            "Make every project surface queryable.",
            "The typed API supports a collaborative UI.",
            created
        ],
    )
    .await
    .unwrap();
    tx.execute(
        "INSERT INTO coordinator_states (challenge_id, status, current_objective, next_step, \
         blocker, worker_id, continuation_id, inbox_cursor, version, heartbeat_at, wake_at, \
         updated_at) VALUES ($1, 'CREATED', $2, 'Frame the first falsifiable hypothesis.', \
         'None', NULL, NULL, 0, 1, NULL, NULL, $3)",
        args![&id, "Make every project surface queryable.", created],
    )
    .await
    .unwrap();
    if let Some(owner) = owner {
        tx.execute(
            "INSERT INTO project_members (challenge_id, subject, role, display_name, email, \
             created_by, created_at, updated_at) VALUES ($1, $2, 'OWNER', $3, $4, $2, $5, $5)",
            args![&id, &owner.subject, &owner.display_name, owner.email.as_deref(), created],
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    id
}

/// One artifact row, like the research service writes.
pub struct ArtifactRow<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub title: &'a str,
    pub status: &'a str,
    pub payload: Value,
    pub hypothesis: Option<&'a str>,
    pub experiment: Option<&'a str>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> ArtifactRow<'a> {
    pub fn hypothesis(id: &'a str, title: &'a str, payload: Value, at: Timestamp) -> Self {
        Self {
            id,
            kind: "H",
            title,
            status: "PROPOSED",
            payload,
            hypothesis: None,
            experiment: None,
            created_at: at,
            updated_at: at,
        }
    }
}

/// Inserts an artifact with its first revision and returns its UID.
pub async fn insert_artifact(db: &Db, challenge_id: &str, row: ArtifactRow<'_>) -> String {
    let uid = new_uuid();
    let mut tx = db.begin_write().await.unwrap();
    tx.execute(
        "INSERT INTO artifacts (uid, challenge_id, artifact_id, kind, title, status, payload, \
         parent_hypothesis_id, parent_experiment_id, version, created_by, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 1, 'Limina', $10, $11)",
        args![
            &uid,
            challenge_id,
            row.id,
            row.kind,
            row.title,
            row.status,
            &row.payload,
            row.hypothesis,
            row.experiment,
            row.created_at,
            row.updated_at
        ],
    )
    .await
    .unwrap();
    tx.execute(
        "INSERT INTO artifact_revisions (artifact_uid, version, status, title, payload, actor, \
         command_id, created_at) VALUES ($1, 1, $2, $3, $4, 'Limina', $5, $6)",
        args![&uid, row.status, row.title, &row.payload, new_uuid(), row.created_at],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    uid
}

/// Adds a later revision of an artifact.
pub async fn insert_revision(db: &Db, uid: &str, version: i32, status: &str, payload: Value) {
    let mut tx = db.begin_write().await.unwrap();
    tx.execute(
        "INSERT INTO artifact_revisions (artifact_uid, version, status, title, payload, actor, \
         command_id, created_at) VALUES ($1, $2, $3, 'Revised', $4, 'Erin Editor', $5, $6)",
        args![uid, version, status, payload, new_uuid(), now()],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

/// A guidance message, as the steering service stores it. Returns its ID.
pub async fn insert_guidance(
    db: &Db,
    challenge_id: &str,
    body: &str,
    status: &str,
    created_at: Timestamp,
    acknowledged_at: Option<Timestamp>,
) -> String {
    let id = new_uuid();
    let mut tx = db.begin_write().await.unwrap();
    tx.execute(
        "INSERT INTO inbox_messages (id, challenge_id, kind, body, actor, status, command_id, \
         created_at, acknowledged_at) VALUES ($1, $2, 'STEER', $3, 'Olivia Owner', $4, $5, $6, $7)",
        args![&id, challenge_id, body, status, new_uuid(), created_at, acknowledged_at],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    id
}

/// An event row (for run details).
pub async fn insert_event(
    db: &Db,
    challenge_id: &str,
    event_type: &str,
    actor: &str,
    payload: Value,
) {
    let mut tx = db.begin_write().await.unwrap();
    tx.execute(
        "INSERT INTO events (id, challenge_id, event_type, actor, artifact_id, payload, \
         command_id, created_at) VALUES ($1, $2, $3, $4, NULL, $5, $6, $7)",
        args![new_uuid(), challenge_id, event_type, actor, payload, new_uuid(), now()],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

/// A finished run with fixed timings and usage (for analytics).
pub struct RunRow<'a> {
    pub status: &'a str,
    pub started_at: Timestamp,
    pub duration_ms: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub reasoning_output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub cost_microusd: Option<i64>,
    pub usage_source: Option<&'a str>,
    pub cost_source: Option<&'a str>,
    pub tool_calls: i32,
}

pub async fn insert_run(db: &Db, challenge_id: &str, row: RunRow<'_>) -> String {
    let id = new_uuid();
    let mut tx = db.begin_write().await.unwrap();
    tx.execute(
        "INSERT INTO runtime_runs (id, challenge_id, runtime_engine, model, status, summary, \
         input_tokens, output_tokens, reasoning_output_tokens, total_tokens, cost_microusd, \
         usage_source, cost_source, tool_calls, retry_count, started_at, completed_at, \
         duration_ms) VALUES ($1, $2, 'codex', 'test-model', $3, '', $4, $5, $6, $7, $8, $9, $10, \
         $11, 0, $12, $13, $14)",
        args![
            &id,
            challenge_id,
            row.status,
            row.input_tokens,
            row.output_tokens,
            row.reasoning_output_tokens,
            row.total_tokens,
            row.cost_microusd,
            row.usage_source,
            row.cost_source,
            row.tool_calls,
            row.started_at,
            row.duration_ms.map(|ms| row.started_at + chrono::Duration::milliseconds(ms)),
            row.duration_ms
        ],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    id
}

/// Event types recorded for a project, oldest first.
pub async fn event_log(
    db: &Db,
    challenge_id: &str,
) -> Vec<(String, String, Option<String>, Value)> {
    let mut tx = db.begin().await.unwrap();
    let rows: Vec<limina_store::models::Event> = tx
        .fetch_all(
            "SELECT * FROM events WHERE challenge_id = $1 ORDER BY sequence",
            args![challenge_id],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    rows.into_iter()
        .map(|row| (row.event_type, row.actor, row.artifact_id, row.payload.0))
        .collect()
}
