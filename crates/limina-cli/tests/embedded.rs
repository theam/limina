//! Embedded mode: `_agent` commands and `db` administration straight on a SQLite database.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::Path;
use std::process::Command;

use limina_cli::client::{LocalRuntimeClient, write_snapshot};
use limina_domain::project::NewChallenge;
use limina_domain::service::ChallengeService;
use limina_store::Db;
use limina_store::schema::HEAD;
use serde_json::Value;
use support::{limina, run};

fn create_project(database_url: &str, slug: &str) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let db = Db::connect(database_url).await.unwrap();
        db.initialize().await.unwrap();
        ChallengeService::new(db.clone(), None)
            .create_challenge(
                NewChallenge {
                    slug: slug.into(),
                    name: "Embedded".into(),
                    objective: "Prove the embedded chain.".into(),
                    success_criteria: "A finding is exported.".into(),
                    ..NewChallenge::default()
                },
                "owner",
                &limina_core::new_uuid(),
            )
            .await
            .unwrap();
        db.close().await;
    });
}

fn agent(database: &str) -> Command {
    let mut command = limina();
    command
        .args(["--database", database, "--actor", "lane-a", "_agent"])
        .env("LIMINA_PROJECT", "embedded");
    command
}

fn ok(command: &mut Command) -> Value {
    let result = run(command);
    assert_eq!(result.code, 0, "{result:?}");
    assert!(result.stdout.ends_with('\n') && result.stdout.lines().count() == 1, "{result:?}");
    result.json()
}

#[test]
fn the_research_chain_runs_and_exports_on_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    // A bare path is a SQLite database, like the Python CLI's `--database`.
    let database = dir.path().join("runtime.db").display().to_string();
    create_project(&format!("sqlite:///{database}"), "embedded");

    let hypothesis = ok(agent(&database).args([
        "hypothesis",
        "add",
        "Caching helps",
        "--statement",
        "A warm cache halves latency.",
        "--mechanism",
        "Fewer disk reads.",
    ]));
    assert_eq!(hypothesis["id"], "H001");
    assert_eq!(hypothesis["status"], "PROPOSED");

    let experiment = ok(agent(&database).args([
        "experiment",
        "add",
        "h001",
        "Warm versus cold",
        "--objective",
        "Measure p50 latency.",
    ]));
    assert_eq!(experiment["id"], "E001");
    assert_eq!(experiment["hypothesis_id"], "H001");

    let claim = ok(agent(&database).args(["experiment", "claim", "e001", "--ttl", "600"]));
    assert_eq!(claim["artifact"]["status"], "RUNNING");
    assert_eq!(claim["lease"]["owner"], "lane-a");

    let observation = ok(agent(&database).args([
        "experiment",
        "observe",
        "E001",
        "p50 fell from 80ms to 41ms.",
        "--evidence",
        "runs/1.json",
    ]));
    assert_eq!(observation["evidence_ref"], "runs/1.json");

    // A stale version is a conflict (exit code 4) and changes nothing.
    let stale = run(agent(&database).args([
        "experiment",
        "complete",
        "E001",
        "--results",
        "R",
        "--analysis",
        "A",
        "--decision",
        "D",
        "--expect",
        "1",
    ]));
    assert_eq!(stale.code, 4, "{stale:?}");
    assert!(stale.stderr.starts_with("version_conflict  "), "{stale:?}");

    let completed = ok(agent(&database).args([
        "experiment",
        "complete",
        "E001",
        "--results",
        "p50 41ms warm, 80ms cold.",
        "--analysis",
        "The cache explains the gap.",
        "--decision",
        "Keep the cache.",
        "--expect",
        "2",
    ]));
    assert_eq!(completed["status"], "COMPLETED");

    let finding = ok(agent(&database).args([
        "finding",
        "publish",
        "e001",
        "Warm caches halve latency",
        "--finding",
        "Latency halves with a warm cache.",
        "--evidence",
        "E001",
        "--impact",
        "medium",
    ]));
    assert_eq!(finding["id"], "F001");
    assert_eq!(finding["payload"]["impact"], "MEDIUM");

    let decided = ok(agent(&database).args([
        "hypothesis",
        "decide",
        "h001",
        "--status",
        "confirmed",
        "--conclusion",
        "Confirmed by E001.",
        "--expect",
        "2",
    ]));
    assert_eq!(decided["status"], "CONFIRMED");

    let shown = ok(agent(&database).args(["show", "e001"]));
    assert_eq!(shown["observations"].as_array().unwrap().len(), 1);
    assert!(shown["lease"].is_null());
    let listed = ok(agent(&database).args(["list", "--kind", "f"]));
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let status = ok(agent(&database).arg("status"));
    assert_eq!(status["counts"]["F"]["PUBLISHED"], 1);

    // Export the knowledge base from the same database.
    let target = dir.path().join("kb-export");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let written = runtime.block_on(async {
        let client = LocalRuntimeClient::connect(&format!("sqlite:///{database}")).await.unwrap();
        let files = client.snapshot("embedded").await.unwrap();
        client.close().await;
        write_snapshot(files, &target).await.unwrap()
    });
    let names: Vec<String> = written
        .iter()
        .map(|path| path.strip_prefix(&target).unwrap().display().to_string())
        .collect();
    assert_eq!(
        names,
        [
            "ACTIVE.md",
            "DASHBOARD.md",
            "mission/CHALLENGE.md",
            "research/experiments/E001-warm-versus-cold.md",
            "research/findings/F001-warm-caches-halve-latency.md",
            "research/hypotheses/H001-caching-helps.md",
        ]
    );
    let finding_note =
        std::fs::read_to_string(target.join("research/findings/F001-warm-caches-halve-latency.md"))
            .unwrap();
    assert!(finding_note.contains("Latency halves with a warm cache."));
    assert!(finding_note.contains("[[E001]]"));
}

#[test]
fn embedded_errors_keep_their_exit_codes() {
    let dir = tempfile::tempdir().unwrap();
    let database = format!("sqlite:///{}", dir.path().join("runtime.db").display());
    create_project(&database, "embedded");

    let missing = run(agent(&database).args(["show", "H404"]));
    assert_eq!(missing.code, 3, "{missing:?}");
    let invalid = run(agent(&database).args(["hypothesis", "add", "T", "--statement", "  "]));
    assert_eq!(invalid.code, 5, "{invalid:?}");
    let lease = run(agent(&database).args(["experiment", "claim", "E001", "--ttl", "5"]));
    assert_eq!(lease.code, 5, "{lease:?}");

    let mut unknown = agent(&database);
    unknown.env("LIMINA_PROJECT", "nope");
    let unknown = run(unknown.args(["--json"]).arg("status"));
    // `--json` after `_agent` is not an option of `status`.
    assert_eq!(unknown.code, 2, "{unknown:?}");
}

fn db(database: &str, args: &[&str]) -> support::Run {
    run(limina().args(["--database", database]).args(args))
}

#[test]
fn project_share_recovers_access_without_a_browser_session() {
    let dir = tempfile::tempdir().unwrap();
    let database = format!("sqlite:///{}", dir.path().join("runtime.db").display());
    create_project(&database, "private");
    let pending = db(
        &database,
        &[
            "--json",
            "project",
            "share",
            "private",
            "--email",
            "reader@example.test",
            "--role",
            "VIEWER",
        ],
    );
    assert_eq!(pending.code, 0, "{pending:?}");
    let value: Value = serde_json::from_str(&pending.stdout).unwrap();
    assert_eq!(value["pending"], true);
    let member = db(
        &database,
        &[
            "--json",
            "project",
            "share",
            "private",
            "--email",
            "reader@example.test",
            "--subject",
            "user_reader",
            "--role",
            "OWNER",
        ],
    );
    assert_eq!(member.code, 0, "{member:?}");
    let value: Value = serde_json::from_str(&member.stdout).unwrap();
    assert_eq!(value["pending"], false);
    assert_eq!(value["subject"], "user_reader");
}

#[test]
fn db_upgrade_and_current_on_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state").join("runtime.db");
    let database = path.display().to_string();
    let url = format!("sqlite:///{database}");

    let empty = db(&database, &["db", "current"]);
    assert_eq!(empty.code, 0, "{empty:?}");
    assert_eq!(empty.stdout, format!("Current revision(s) for {url}:\n"));

    let upgraded = db(&database, &["--json", "db", "upgrade"]);
    assert_eq!(upgraded.code, 0, "{upgraded:?}");
    assert_eq!(
        upgraded.stdout,
        format!("{{\n  \"database\": \"{url}\",\n  \"revision\": \"head\"\n}}\n")
    );
    assert!(Path::new(&path).is_file());

    // Upgrading again is a no-op.
    let again = db(&url, &["db", "upgrade"]);
    assert_eq!(again.code, 0, "{again:?}");
    assert!(again.stdout.contains("head"));

    let current = db(&url, &["db", "current"]);
    assert_eq!(current.code, 0, "{current:?}");
    let lines: Vec<&str> = current.stdout.lines().collect();
    assert_eq!(lines[0], format!("Current revision(s) for {url}:"));
    assert_eq!(lines[1], format!("Rev: {HEAD} (head)"));
    assert!(lines[2].starts_with("Parent: "));
}

#[test]
fn an_unreachable_database_is_reported_not_panicked() {
    let dir = tempfile::tempdir().unwrap();
    // A directory where the database file should be.
    let blocked = dir.path().join("blocked.db");
    std::fs::create_dir(&blocked).unwrap();
    let result = db(&blocked.display().to_string(), &["--json", "db", "upgrade"]);
    assert_eq!(result.code, 8, "{result:?}");
    assert_eq!(result.json()["error"]["code"], "database_error");
}

#[test]
fn a_file_based_project_imports_as_a_draft() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("My Search Lab");
    let write = |path: &str, text: &str| {
        let path = project.join("kb").join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(
        "mission/CHALLENGE.md",
        "# Research Mission\n\n## Objective\n\nFind it.\n\n## Success Criteria\n\nFound.\n",
    );
    write(
        "research/hypotheses/H001-a.md",
        "---\nid: H001\nstatus: PROPOSED\n---\n\n# H001 — A\n\n## Statement\n\nS.\n",
    );
    write(
        "research/experiments/E001-b.md",
        "---\nid: E001\nstatus: DESIGNED\nhypothesis: H001\n---\n\n# E001 — B\n\n## Objective\n\nO.\n",
    );
    let database = dir.path().join("runtime.db").display().to_string();

    let mut command = limina();
    command.args(["--database", &database, "--json", "project", "import"]).arg(&project);
    let result = run(&mut command);
    assert_eq!(result.code, 0, "{result:?}");
    let report = result.json();
    assert_eq!(report["slug"], "my-search-lab");
    assert_eq!(report["name"], "my-search-lab", "a generic mission title falls back to the slug");
    assert_eq!((report["hypotheses"].as_i64(), report["experiments"].as_i64()), (Some(1), Some(1)));

    let mut status = agent(&database);
    status.env("LIMINA_PROJECT", "my-search-lab").args(["show", "E001"]);
    assert_eq!(ok(&mut status)["hypothesis_id"], "H001");

    let mut again = limina();
    again.args(["--database", &database, "--json", "project", "import"]).arg(project.join("kb"));
    let result = run(&mut again);
    assert_ne!(result.code, 0, "the slug already exists: {result:?}");
}
