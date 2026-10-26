#![allow(clippy::unwrap_used)]

use std::path::Path;

use limina_domain::exporter::MarkdownExporter;
use limina_domain::import::{ImportOptions, import_knowledge_base};
use limina_domain::research::NewHypothesis;
use limina_domain::service::ChallengeService;
use limina_store::Db;

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn knowledge_base(root: &Path) {
    write(
        root,
        "mission/CHALLENGE.md",
        "---\ntype: mission\n---\n\n# Retrieval reliability\n\n## Objective\n\nCut unsupported citations.\n\n## Context\n\nFrozen generator.\n\n## Success Criteria\n\nBelow 8% on the test split.\n\n## Constraints\n\n- No new data.\n\n## Links\n\n- [[ACTIVE]]\n",
    );
    write(
        root,
        "ACTIVE.md",
        "# Active State\n\n## Current Objective\n\nTune the threshold.\n\n## Standing Directives\n\nAlways report both metrics.\n\n## Next Step\n\nRun E002 on the test split.\n\n## Blocker\n\nNone — the owner approved option A on 2026-04-20.\n\n## Links\n\n- [[CHALLENGE]]\n",
    );
    write(
        root,
        "research/hypotheses/H001-reranking-helps.md",
        "---\nid: \"H001\"\nstatus: CONFIRMED\ncreated: \"2026-04-15\"\nlast_updated: \"2026-04-20\"\n---\n\n# H001 — Reranking helps\n\n> **Status**: CONFIRMED\n\n## Statement\n\nReranking cuts unsupported citations.\n\n## Mechanism\n\nBetter passages.\n\n## Evidence\n\nSee [[E002]].\n\n## Conclusion\n\nConfirmed.\n\n## Links\n\n- [[E001]]\n",
    );
    write(
        root,
        "research/hypotheses/H002-threshold.md",
        "---\nid: \"H002\"\nstatus: TESTED\ncreated: \"2026-04-21\"\n---\n\n# H002 — Abstention threshold\n\n## Statement\n\nA threshold keeps the answer rate.\n",
    );
    write(
        root,
        "research/experiments/E001-baseline.md",
        "---\nid: \"E001\"\nstatus: IN_PROGRESS\nhypothesis: \"H001\"\ncreated: \"2026-04-16\"\n---\n\n# E001 — Baseline\n\n## Objective\n\nMeasure the baseline.\n\n## Setup\n\nDev split.\n\n## Procedure\n\nRun the eval.\n\n## Progress\n\nHalf done.\n",
    );
    write(
        root,
        "research/experiments/E002-rerank.md",
        "---\nid: \"E002\"\nstatus: COMPLETED\nhypothesis: \"[[H001]]\"\ncreated: \"2026-04-17\"\ncompleted: \"2026-04-19\"\n---\n\n# E002 — Rerank\n\n## Objective\n\nTry a cross-encoder.\n\n## Results\n\n6.1% unsupported.\n\n## Decision\n\nAdopt it.\n",
    );
    write(
        root,
        "research/findings/F001-rerank-works.md",
        "---\nid: \"F001\"\nexperiment: \"E002\"\nimpact: \"high - halves unsupported citations\"\ncreated: \"2026-04-19\"\n---\n\n# F001 — Reranking works\n\n## The re-judge, by the numbers\n\n6.1% vs 11.4%.\n\n## Evidence\n\nE002 results.\n\n## Next Move\n\nTune the threshold.\n",
    );
    write(
        root,
        "research/findings/F002-orphan.md",
        "---\nid: \"F002\"\nexperiment: \"E099\"\nhypothesis: \"H001\"\nimpact: LOW\n---\n\n# F002 — Orphan\n\n## Finding\n\nNo experiment.\n",
    );
    write(root, "research/hypotheses/README.md", "# Not a note\n");
}

#[tokio::test]
async fn a_file_based_project_imports_with_its_ids_links_and_state() {
    let dir = tempfile::tempdir().unwrap();
    let kb = dir.path().join("kb");
    knowledge_base(&kb);
    let db = Db::connect(&format!("sqlite:///{}", dir.path().join("runtime.db").display()))
        .await
        .unwrap();
    db.initialize().await.unwrap();
    let service = ChallengeService::new(db, None);

    let report = import_knowledge_base(
        &service,
        &kb,
        ImportOptions {
            slug: "retrieval".into(),
            name: None,
            runtime_engine: Some("claude-code".into()),
            owner: None,
            actor: "Importer".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(report.name, "Retrieval reliability");
    assert_eq!((report.hypotheses, report.experiments, report.findings), (2, 2, 1));
    let skipped: Vec<_> = report.skipped.iter().map(|s| s.path.as_str()).collect();
    assert_eq!(skipped, ["README.md", "F002"]);
    assert!(report.notes.iter().any(|n| n == "H002: status TESTED imported as TESTING."));
    assert!(report.notes.iter().any(|n| n == "E001: status IN_PROGRESS imported as RUNNING."));

    let project = service.get_challenge("retrieval").await.unwrap();
    let project = serde_json::to_value(project).unwrap();
    assert_eq!(project["runtime_engine"], "claude-code");
    assert_eq!(project["objective"], "Cut unsupported citations.");
    assert_eq!(project["success_criteria"], "Below 8% on the test split.");
    let context = project["context"].as_str().unwrap();
    assert!(context.contains("### Constraints\n\n- No new data."));
    assert!(context.contains("### Standing Directives\n\nAlways report both metrics."));
    assert_eq!(project["coordinator"]["status"], "CREATED");
    assert_eq!(project["coordinator"]["current_objective"], "Tune the threshold.");
    assert_eq!(project["coordinator"]["next_step"], "Run E002 on the test split.");
    assert_eq!(project["coordinator"]["blocker"], "None", "\"None — context\" is no blocker");
    assert!(context.contains("### Blocker notes\n\nthe owner approved option A on 2026-04-20."));

    let h001 =
        serde_json::to_value(service.get_artifact("retrieval", "H001").await.unwrap()).unwrap();
    let h001 = if h001.get("artifact").is_some() { h001["artifact"].clone() } else { h001 };
    assert_eq!(h001["status"], "CONFIRMED");
    assert_eq!(h001["title"], "Reranking helps");
    assert_eq!(
        h001["payload"]["conclusion"],
        "### Evidence\n\nSee [[E002]].\n\n### Conclusion\n\nConfirmed."
    );
    let e001 =
        serde_json::to_value(service.get_artifact("retrieval", "E001").await.unwrap()).unwrap();
    let e001 = if e001.get("artifact").is_some() { e001["artifact"].clone() } else { e001 };
    assert_eq!(
        e001["payload"]["procedure"],
        "### Setup\n\nDev split.\n\n### Procedure\n\nRun the eval."
    );
    assert_eq!(e001["payload"]["results"], "### Progress\n\nHalf done.");
    let f001 =
        serde_json::to_value(service.get_artifact("retrieval", "F001").await.unwrap()).unwrap();
    let f001 = if f001.get("artifact").is_some() { f001["artifact"].clone() } else { f001 };
    assert_eq!(f001["hypothesis_id"], "H001", "the hypothesis comes from the experiment");
    assert_eq!(f001["payload"]["impact"], "HIGH");
    assert_eq!(f001["payload"]["improvement"], "### Impact\n\nhalves unsupported citations");
    assert_eq!(f001["payload"]["finding"], "### The re-judge, by the numbers\n\n6.1% vs 11.4%.");

    // New knowledge continues after the imported IDs, and the vault renders.
    let next = service
        .create_hypothesis(
            "retrieval",
            NewHypothesis {
                title: "Next".into(),
                statement: "S".into(),
                mechanism: "M".into(),
                generalization: "G".into(),
                shortcut_risks: "R".into(),
                test_plan: "T".into(),
            },
            "Ada",
            "cmd-next",
        )
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(next).unwrap()["id"], "H003");
    let files = MarkdownExporter::new(service.clone()).snapshot("retrieval").await.unwrap();
    assert!(files.keys().any(|path| path.contains("F001")));
    assert!(files.values().any(|text| text.contains("6.1% vs 11.4%.")));

    let again = import_knowledge_base(
        &service,
        &kb,
        ImportOptions {
            slug: "retrieval".into(),
            name: None,
            runtime_engine: None,
            owner: None,
            actor: "Importer".into(),
        },
    )
    .await;
    assert!(again.is_err(), "importing into an existing slug fails instead of duplicating");
}
