//! Shared helpers for the service tests. Every test runs on SQLite, and also on PostgreSQL when
//! `LIMINA_TEST_POSTGRES_URL` points at a database the tests may reset (its `public` schema is
//! dropped and migrated before each test), like `limina-store/tests/backends.rs`:
//!
//! ```sh
//! LIMINA_TEST_POSTGRES_URL=postgres://limina:limina@127.0.0.1:55432/limina cargo test -p limina-domain
//! ```

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::future::Future;
use std::sync::Arc;

use limina_core::new_uuid;
use limina_domain::project::NewChallenge;
use limina_domain::research::{NewExperiment, NewHypothesis};
use limina_domain::service::{ArtifactView, ChallengeService};
use limina_store::{Db, SecretCipher};
use tokio::sync::{Mutex, MutexGuard};

/// Tests in one binary run in parallel; PostgreSQL tests share one database, so they take turns.
static POSTGRES: Mutex<()> = Mutex::const_new(());

/// One database under test.
#[derive(Debug, Clone)]
pub struct Backend {
    pub name: &'static str,
    pub db: Db,
}

/// What must outlive a test: the SQLite directory, or the PostgreSQL turn. Held by the caller
/// rather than the [`Backend`], because a test's `async move` block may capture only `backend.db`.
pub struct Hold {
    _dir: Option<tempfile::TempDir>,
    _turn: Option<MutexGuard<'static, ()>>,
}

impl Backend {
    /// A service with an ephemeral secret cipher.
    pub fn service(&self) -> ChallengeService {
        ChallengeService::new(self.db.clone(), Some(Arc::new(SecretCipher::ephemeral())))
    }

    pub fn is_postgres(&self) -> bool {
        self.name == "postgres"
    }
}

pub async fn sqlite() -> (Backend, Hold) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::connect(&format!("sqlite:///{}", dir.path().join("runtime.db").display()))
        .await
        .unwrap();
    db.initialize().await.unwrap();
    (Backend { name: "sqlite", db }, Hold { _dir: Some(dir), _turn: None })
}

/// A freshly migrated PostgreSQL database, when one is configured.
pub async fn postgres() -> Option<(Backend, Hold)> {
    let url = std::env::var("LIMINA_TEST_POSTGRES_URL").ok()?;
    let turn = POSTGRES.lock().await;
    let db = Db::connect(&url).await.unwrap();
    let mut tx = db.begin_write().await.unwrap();
    tx.execute_script("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").await.unwrap();
    tx.commit().await.unwrap();
    db.upgrade().await.unwrap();
    db.initialize().await.unwrap();
    Some((Backend { name: "postgres", db }, Hold { _dir: None, _turn: Some(turn) }))
}

/// Runs `test` on SQLite, then on PostgreSQL when configured.
pub async fn each_backend<F, Fut>(test: F)
where
    F: Fn(Backend) -> Fut,
    Fut: Future<Output = ()>,
{
    let (backend, hold) = sqlite().await;
    let db = backend.db.clone();
    test(backend).await;
    db.close().await;
    drop(hold);
    if let Some((backend, hold)) = postgres().await {
        let db = backend.db.clone();
        test(backend).await;
        db.close().await;
        drop(hold);
    }
}

pub fn command_id() -> String {
    new_uuid()
}

/// The project every runtime test starts from (the Python suite's `setUp`).
pub async fn retrieval(service: &ChallengeService) {
    service
        .create_challenge(
            NewChallenge {
                slug: "retrieval".into(),
                name: "Retrieval quality".into(),
                objective: "Improve retrieval quality.".into(),
                context: "Existing hybrid baseline.".into(),
                success_criteria: "Improve NDCG without a latency regression.".into(),
                ..NewChallenge::default()
            },
            "owner",
            &command_id(),
        )
        .await
        .unwrap();
}

pub fn hypothesis(title: &str) -> NewHypothesis {
    NewHypothesis {
        title: title.into(),
        statement: "A stronger reranker improves NDCG.".into(),
        mechanism: "Pairwise relevance modeling.".into(),
        generalization: "The mechanism is query independent.".into(),
        shortcut_risks: "Overfitting the fixture.".into(),
        test_plan: "Use a frozen candidate set.".into(),
    }
}

pub fn experiment(hypothesis_id: &str, title: &str) -> NewExperiment {
    NewExperiment {
        hypothesis_id: hypothesis_id.into(),
        title: title.into(),
        objective: "Measure NDCG and latency.".into(),
        procedure: "Run baseline and treatment.".into(),
        success_criteria: "At least 10 percent NDCG improvement.".into(),
        guardrails: "Use identical candidates and hardware.".into(),
    }
}

pub async fn create_hypothesis(service: &ChallengeService, title: &str) -> ArtifactView {
    service
        .create_hypothesis("retrieval", hypothesis(title), "researcher", &command_id())
        .await
        .unwrap()
}

pub async fn create_experiment(
    service: &ChallengeService,
    hypothesis_id: &str,
    title: &str,
) -> ArtifactView {
    service
        .create_experiment(
            "retrieval",
            experiment(hypothesis_id, title),
            "researcher",
            &command_id(),
        )
        .await
        .unwrap()
}

/// The error code of a failed call.
pub fn code<T: std::fmt::Debug>(result: limina_core::Result<T>) -> &'static str {
    result.expect_err("the call should fail").code()
}
