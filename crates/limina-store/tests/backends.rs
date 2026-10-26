//! The same queries on SQLite and PostgreSQL. PostgreSQL runs when `LIMINA_TEST_POSTGRES_URL`
//! points at an empty database, for example:
//!
//! ```sh
//! docker run -d --name limina-pg -e POSTGRES_PASSWORD=limina -e POSTGRES_USER=limina \
//!   -p 127.0.0.1:55432:5432 postgres:17-alpine
//! LIMINA_TEST_POSTGRES_URL=postgres://limina:limina@127.0.0.1:55432/limina cargo test -p limina-store
//! ```

#![allow(clippy::unwrap_used)]

use limina_core::{new_uuid, now};
use limina_store::models::{Challenge, Event, LiveTicket, RuntimeRun};
use limina_store::schema::HEAD;
use limina_store::{Db, args};
use serde_json::json;

async fn backends() -> Vec<(Db, Option<tempfile::TempDir>)> {
    let dir = tempfile::tempdir().unwrap();
    let sqlite = Db::connect(&format!("sqlite:///{}", dir.path().join("runtime.db").display()))
        .await
        .unwrap();
    let mut all = vec![(sqlite, Some(dir))];
    if let Ok(url) = std::env::var("LIMINA_TEST_POSTGRES_URL") {
        let db = Db::connect(&url).await.unwrap();
        let mut tx = db.begin_write().await.unwrap();
        tx.execute_script("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").await.unwrap();
        tx.commit().await.unwrap();
        all.push((db, None));
    }
    all
}

#[tokio::test]
async fn one_dialect_runs_on_every_backend() {
    for (db, _dir) in backends().await {
        let applied = db.upgrade().await.unwrap();
        if matches!(db, Db::Postgres(_)) {
            assert_eq!(applied.len(), limina_store::schema::POSTGRES_MIGRATIONS.len());
            assert!(db.upgrade().await.unwrap().is_empty(), "upgrade is idempotent");
        }
        db.initialize().await.unwrap();
        assert_eq!(db.revision().await.unwrap().as_deref(), Some(HEAD));

        let id = new_uuid();
        let created = now();
        let mut tx = db.begin_write().await.unwrap();
        tx.execute(
            "INSERT INTO challenges (id, slug, name, objective, context, success_criteria, \
             runtime_engine, status, version, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, '', $5, 'codex', 'ACTIVE', 1, $6, $6)",
            args![&id, "demo", "Demo", "Find out.", "Know.", created],
        )
        .await
        .unwrap();
        for (index, body) in ["first", "second"].iter().enumerate() {
            tx.execute(
                "INSERT INTO events (id, challenge_id, event_type, actor, artifact_id, payload, \
                 command_id, created_at) VALUES ($1, $2, 'runtime.codex', 'Limina', NULL, $3, $4, $5)",
                args![new_uuid(), &id, json!({"text": body, "n": index}), new_uuid(), now()],
            )
            .await
            .unwrap();
        }
        tx.execute(
            "INSERT INTO live_tickets (token_hash, challenge_id, subject, display_name, role, \
             instance_admin, expires_at, used_at, created_at) VALUES ($1, $2, 's', 'S', 'OWNER', $3, $4, NULL, $4)",
            args!["hash", &id, true, created],
        )
        .await
        .unwrap();
        tx.execute(
            "INSERT INTO runtime_runs (id, challenge_id, runtime_engine, model, status, summary, \
             input_tokens, total_tokens, tool_calls, retry_count, started_at, duration_ms) \
             VALUES ($1, $2, 'codex', NULL, 'SUCCEEDED', '', $3, NULL, 2, 0, $4, $5)",
            args![new_uuid(), &id, 12_345_678_901_i64, created, 1500_i64],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let mut tx = db.begin().await.unwrap();
        let sql = format!("SELECT * FROM challenges WHERE slug = $1{}", tx.for_update());
        let challenge: Challenge = tx.fetch_one(&sql, args!["demo"]).await.unwrap();
        assert_eq!(challenge.id, id);
        assert_eq!(challenge.created_at, created, "timestamps round-trip to the microsecond");

        let events: Vec<Event> = tx
            .fetch_all(
                "SELECT * FROM events WHERE challenge_id = $1 AND sequence > $2 ORDER BY sequence",
                args![&id, 0_i64],
            )
            .await
            .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].payload.0["text"], "second");

        let count: i64 = tx
            .scalar(
                "SELECT CAST(COUNT(*) AS BIGINT) FROM events WHERE challenge_id = $1",
                args![&id],
            )
            .await
            .unwrap();
        assert_eq!(count, 2);
        let last: Option<i64> = tx
            .scalar(
                "SELECT CAST(MAX(sequence) AS BIGINT) FROM events WHERE challenge_id = $1",
                args![&id],
            )
            .await
            .unwrap();
        assert_eq!(last, Some(i64::from(events[1].sequence)));

        let ticket: LiveTicket = tx
            .fetch_one("SELECT * FROM live_tickets WHERE token_hash = $1", args!["hash"])
            .await
            .unwrap();
        assert!(ticket.instance_admin);
        assert!(ticket.used_at.is_none());
        let expired: i64 = tx
            .scalar(
                "SELECT CAST(COUNT(*) AS BIGINT) FROM live_tickets WHERE expires_at < $1",
                args![now() + chrono::Duration::seconds(1)],
            )
            .await
            .unwrap();
        assert_eq!(expired, 1, "timestamps compare in SQL on both backends");

        let run: RuntimeRun = tx.fetch_one("SELECT * FROM runtime_runs", args![]).await.unwrap();
        assert_eq!(run.input_tokens, Some(12_345_678_901));
        assert_eq!(run.total_tokens, None);
        assert_eq!(run.duration_ms, Some(1500));
        tx.commit().await.unwrap();

        let mut tx = db.begin_write().await.unwrap();
        let duplicate = tx
            .execute(
                "INSERT INTO challenges (id, slug, name, objective, context, success_criteria, \
                 runtime_engine, status, version, created_at, updated_at) \
                 VALUES ($1, 'demo', 'Again', 'x', '', 'y', 'codex', 'ACTIVE', 1, $2, $2)",
                args![new_uuid(), now()],
            )
            .await
            .unwrap_err();
        assert!(duplicate.is_unique_violation());
        tx.rollback().await.unwrap();

        let mut tx = db.begin_write().await.unwrap();
        let next: i32 = tx
            .scalar(
                "INSERT INTO id_counters (challenge_id, kind, next_value) VALUES ($1, $2, 2) \
                 ON CONFLICT(challenge_id, kind) DO UPDATE SET next_value = id_counters.next_value + 1 \
                 RETURNING next_value - 1",
                args![&id, "H"],
            )
            .await
            .unwrap();
        assert_eq!(next, 1);
        tx.commit().await.unwrap();
        db.close().await;
    }
}

#[tokio::test]
async fn sqlite_adds_model_defaults_to_existing_projects_without_changing_them() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::connect(&format!("sqlite:///{}", dir.path().join("legacy.db").display()))
        .await
        .unwrap();
    let old_schema = include_str!("../schema/sqlite.sql").replacen(
        "\tmodel VARCHAR(200),\n\treasoning_effort VARCHAR(64),\n",
        "",
        1,
    );
    // Match the schema's whitespace while proving it really is a legacy table.
    let old_schema =
        old_schema.replace("\tmodel VARCHAR(200),\n\treasoning_effort VARCHAR(64), \n", "");
    let mut tx = db.begin_write().await.unwrap();
    tx.execute_script(&old_schema).await.unwrap();
    tx.execute("INSERT INTO challenges (id, slug, name, objective, context, success_criteria, runtime_engine, status, version, created_at, updated_at) VALUES ('legacy','legacy','Legacy','Mission','','Success','codex','ACTIVE',1,$1,$1)", args![now()]).await.unwrap();
    let columns: i64 = tx.scalar("SELECT CAST(COUNT(*) AS BIGINT) FROM pragma_table_info('challenges') WHERE name IN ('model','reasoning_effort')", vec![]).await.unwrap();
    assert_eq!(columns, 0, "fixture must predate project model columns");
    tx.commit().await.unwrap();
    db.initialize().await.unwrap();
    db.initialize().await.unwrap();
    let mut tx = db.begin().await.unwrap();
    let project: Challenge =
        tx.fetch_one("SELECT * FROM challenges WHERE slug = 'legacy'", vec![]).await.unwrap();
    assert_eq!(project.name, "Legacy");
    assert_eq!(project.model, None);
    assert_eq!(project.reasoning_effort, None);
    tx.commit().await.unwrap();
    db.close().await;
}
