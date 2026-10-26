//! Imports a file-based Limina project (`kb/`) into the runtime.
//!
//! The mission becomes the project's objective, success criteria and context; `ACTIVE.md` becomes
//! the coordinator state; hypotheses, experiments and findings keep their IDs, titles, statuses,
//! dates and links, so `[[H001]]`-style references in their text still resolve. Known template
//! sections map to artifact fields; any other section is kept, under its own heading, in the
//! field that holds the note's narrative, so no written content is lost.
//!
//! The project is created as a draft: nothing runs until someone starts it.

mod markdown;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use limina_core::{LiminaError, Result, Timestamp, new_uuid, now};
use limina_store::args;
use limina_store::models::Artifact;
use serde::Serialize;
use serde_json::{Value, json};

pub use markdown::Document;
use markdown::{artifact_reference, strip_artifact_prefix};

use crate::common::{find_challenge, record_event};
use crate::project::{NewChallenge, ProjectOwner};
use crate::service::ChallengeService;
use crate::service::internal::record_revision;

/// What to call the imported project and who owns it.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    pub slug: String,
    /// Defaults to the mission's `#` heading, or the slug when that heading is generic.
    pub name: Option<String>,
    /// `codex` (default) or `claude-code`.
    pub runtime_engine: Option<String>,
    pub owner: Option<ProjectOwner>,
    pub actor: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportReport {
    pub slug: String,
    pub name: String,
    pub hypotheses: usize,
    pub experiments: usize,
    pub findings: usize,
    /// Notes that were not imported, with the reason.
    pub skipped: Vec<SkippedNote>,
    /// Adjustments made while importing (renamed statuses, dropped links, …).
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkippedNote {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
struct Note {
    artifact_id: String,
    kind: &'static str,
    title: String,
    status: String,
    payload: Value,
    parent_hypothesis_id: Option<String>,
    parent_experiment_id: Option<String>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

/// Reads `kb_dir` (the folder that contains `mission/CHALLENGE.md`) and creates the project.
pub async fn import_knowledge_base(
    service: &ChallengeService,
    kb_dir: &Path,
    options: ImportOptions,
) -> Result<ImportReport> {
    let kb = KnowledgeBase::read(kb_dir).await?;
    let mut report =
        ImportReport { slug: options.slug.trim().to_lowercase(), ..Default::default() };
    let notes = kb.notes(&mut report);
    report.name = options
        .name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| kb.mission_name())
        .unwrap_or_else(|| report.slug.clone());

    let created = service
        .create_challenge(
            NewChallenge {
                slug: report.slug.clone(),
                name: report.name.clone(),
                objective: kb.objective(),
                success_criteria: kb.success_criteria(),
                context: kb.context(),
                runtime_engine: options.runtime_engine.clone(),
                owner: options.owner.clone(),
                ..Default::default()
            },
            &options.actor,
            &format!("import:{}:create", report.slug),
        )
        .await?;

    let mut tx = service.db().begin_write().await?;
    let challenge = find_challenge(&mut tx, &created.slug).await?;
    let mut highest: BTreeMap<&str, i64> = BTreeMap::new();
    for note in &notes {
        let artifact: Artifact = tx
            .fetch_one(
                "INSERT INTO artifacts (uid, challenge_id, artifact_id, kind, title, status, \
                 payload, parent_hypothesis_id, parent_experiment_id, version, created_by, \
                 created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 1, $10, $11, $12) RETURNING *",
                args![
                    new_uuid(),
                    &challenge.id,
                    &note.artifact_id,
                    note.kind,
                    &note.title,
                    &note.status,
                    &note.payload,
                    note.parent_hypothesis_id.as_deref(),
                    note.parent_experiment_id.as_deref(),
                    &options.actor,
                    note.created_at,
                    note.updated_at
                ],
            )
            .await?;
        record_revision(
            &mut tx,
            &artifact,
            &options.actor,
            &format!("import:{}", note.artifact_id),
        )
        .await?;
        let number: i64 = note.artifact_id[1..].parse().unwrap_or(0);
        let entry = highest.entry(note.kind).or_default();
        *entry = (*entry).max(number);
        match note.kind {
            "H" => report.hypotheses += 1,
            "E" => report.experiments += 1,
            _ => report.findings += 1,
        }
    }
    for (kind, number) in highest {
        tx.execute(
            "INSERT INTO id_counters (challenge_id, kind, next_value) VALUES ($1, $2, $3) \
             ON CONFLICT(challenge_id, kind) DO UPDATE SET next_value = $3",
            args![&challenge.id, kind, i32::try_from(number + 1).unwrap_or(i32::MAX)],
        )
        .await?;
    }
    let state = kb.active_state();
    tx.execute(
        "UPDATE coordinator_states SET current_objective = $1, next_step = $2, blocker = $3, \
         version = version + 1, updated_at = $4 WHERE challenge_id = $5",
        args![&state.current_objective, &state.next_step, &state.blocker, now(), &challenge.id],
    )
    .await?;
    let summary = format!(
        "Imported {} hypotheses, {} experiments and {} findings from a knowledge base.",
        report.hypotheses, report.experiments, report.findings
    );
    record_event(
        &mut tx,
        &challenge.id,
        "project.imported",
        &options.actor,
        &format!("import:{}:knowledge", report.slug),
        json!({
            "summary": summary,
            "hypotheses": report.hypotheses,
            "experiments": report.experiments,
            "findings": report.findings,
            "skipped": report.skipped.len(),
        }),
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(report)
}

/// The parsed contents of a `kb/` folder.
struct KnowledgeBase {
    mission: Document,
    active: Option<Document>,
    notes: Vec<(PathBuf, Document)>,
}

impl KnowledgeBase {
    async fn read(kb_dir: &Path) -> Result<Self> {
        let mission_path = kb_dir.join("mission").join("CHALLENGE.md");
        let mission = read_document(&mission_path).await?.ok_or_else(|| {
            LiminaError::invariant("This folder is not a Limina knowledge base.")
                .with("expected", "mission/CHALLENGE.md")
                .with("path", kb_dir.display().to_string())
        })?;
        let active = read_document(&kb_dir.join("ACTIVE.md")).await?;
        let mut notes = Vec::new();
        for folder in ["research/hypotheses", "research/experiments", "research/findings"] {
            let directory = kb_dir.join(folder);
            let Ok(mut entries) = tokio::fs::read_dir(&directory).await else { continue };
            let mut paths = Vec::new();
            while let Some(entry) = entries.next_entry().await.map_err(io_error)? {
                let path = entry.path();
                if path.extension().is_some_and(|extension| extension == "md") {
                    paths.push(path);
                }
            }
            paths.sort();
            for path in paths {
                if let Some(document) = read_document(&path).await? {
                    notes.push((path, document));
                }
            }
        }
        Ok(Self { mission, active, notes })
    }

    fn mission_name(&self) -> Option<String> {
        let title = self.mission.title.as_deref()?.trim();
        let generic = ["research mission", "mission", "search mission", "challenge"];
        (!title.is_empty() && !generic.contains(&title.to_lowercase().as_str()))
            .then(|| title.to_owned())
    }

    fn objective(&self) -> String {
        self.mission
            .section("Objective")
            .filter(|text| !text.is_empty())
            .map_or_else(|| self.mission.preamble.clone(), str::to_owned)
    }

    fn success_criteria(&self) -> String {
        self.mission.section("Success Criteria").unwrap_or_default().to_owned()
    }

    /// Every other mission section, then the working notes from `ACTIVE.md` that have no
    /// field of their own, so standing directives reach the engine.
    fn context(&self) -> String {
        let mut parts: Vec<String> = self
            .mission
            .sections
            .iter()
            .filter(|(title, body)| {
                !body.is_empty()
                    && !["objective", "success criteria", "links"]
                        .contains(&title.to_lowercase().as_str())
            })
            .map(|(title, body)| format!("### {title}\n\n{body}"))
            .collect();
        if let Some(active) = &self.active {
            let working: Vec<String> = active
                .sections
                .iter()
                .filter(|(title, body)| {
                    !body.is_empty()
                        && !["current objective", "current phase", "next step", "blocker", "links"]
                            .contains(&title.to_lowercase().as_str())
                })
                .map(|(title, body)| format!("### {title}\n\n{body}"))
                .collect();
            let mut working = working;
            if let Some((_, Some(notes))) = self.active_blocker() {
                working.push(format!("### Blocker notes\n\n{notes}"));
            }
            if !working.is_empty() {
                parts.push(format!("## Working notes (imported)\n\n{}", working.join("\n\n")));
            }
        }
        parts.join("\n\n")
    }

    /// The `Blocker` section as (blocker, notes). "None — some context" is no blocker; its
    /// context is kept as working notes.
    fn active_blocker(&self) -> Option<(String, Option<String>)> {
        let text = self.active.as_ref()?.section("Blocker")?.trim();
        if text.is_empty() {
            return None;
        }
        let Some(rest) =
            text.get(..4).filter(|head| head.eq_ignore_ascii_case("none")).map(|_| &text[4..])
        else {
            return Some((text.to_owned(), None));
        };
        if rest.starts_with(|c: char| c.is_alphanumeric()) {
            return Some((text.to_owned(), None));
        }
        let notes = rest.trim_start_matches([' ', '.', ',', ':', ';', '-', '—', '–']).trim();
        Some(("None".to_owned(), (!notes.is_empty()).then(|| notes.to_owned())))
    }

    fn active_state(&self) -> ActiveState {
        let section = |name: &str| {
            self.active
                .as_ref()
                .and_then(|doc| doc.section(name))
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        let objective = section("Current Objective").unwrap_or_else(|| self.objective());
        let current_objective = match section("Current Phase") {
            Some(phase) => format!("{objective}\n\n{phase}"),
            None => objective,
        };
        ActiveState {
            current_objective,
            next_step: section("Next Step").unwrap_or_else(|| {
                "Review the imported knowledge and choose the next step.".into()
            }),
            blocker: match self.active_blocker() {
                Some((blocker, _)) => blocker,
                None => section("Blocker").unwrap_or_else(|| "None".to_owned()),
            },
        }
    }

    fn notes(&self, report: &mut ImportReport) -> Vec<Note> {
        let mut parsed: Vec<Note> = Vec::new();
        let mut seen = BTreeSet::new();
        for (path, document) in &self.notes {
            let display =
                path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match note_from(path, document, report) {
                Ok(note) if seen.insert(note.artifact_id.clone()) => parsed.push(note),
                Ok(note) => report.skipped.push(SkippedNote {
                    path: display,
                    reason: format!("{} appears more than once.", note.artifact_id),
                }),
                Err(reason) => report.skipped.push(SkippedNote { path: display, reason }),
            }
        }
        // Links must point at imported notes; findings need both parents to render.
        let ids: BTreeSet<String> = parsed.iter().map(|note| note.artifact_id.clone()).collect();
        let experiment_parent: BTreeMap<String, Option<String>> = parsed
            .iter()
            .filter(|note| note.kind == "E")
            .map(|note| (note.artifact_id.clone(), note.parent_hypothesis_id.clone()))
            .collect();
        let mut kept = Vec::new();
        for mut note in parsed {
            if let Some(parent) = note.parent_hypothesis_id.clone()
                && !ids.contains(&parent)
            {
                report
                    .notes
                    .push(format!("{}: dropped the link to missing {parent}.", note.artifact_id));
                note.parent_hypothesis_id = None;
            }
            if note.kind == "F" {
                let experiment = note.parent_experiment_id.clone().filter(|id| ids.contains(id));
                let Some(experiment) = experiment else {
                    report.skipped.push(SkippedNote {
                        path: note.artifact_id.clone(),
                        reason: "A finding needs an imported experiment.".to_owned(),
                    });
                    continue;
                };
                if note.parent_hypothesis_id.is_none() {
                    note.parent_hypothesis_id =
                        experiment_parent.get(&experiment).cloned().flatten();
                }
                if note.parent_hypothesis_id.is_none() {
                    report.skipped.push(SkippedNote {
                        path: note.artifact_id.clone(),
                        reason: "A finding needs a hypothesis, directly or through its experiment."
                            .to_owned(),
                    });
                    continue;
                }
            }
            kept.push(note);
        }
        kept.sort_by(|a, b| (a.kind, &a.artifact_id).cmp(&(b.kind, &b.artifact_id)));
        // Hypotheses first, then experiments, then findings, so parents exist before children.
        kept.sort_by_key(|note| match note.kind {
            "H" => 0,
            "E" => 1,
            _ => 2,
        });
        kept
    }
}

struct ActiveState {
    current_objective: String,
    next_step: String,
    blocker: String,
}

/// Maps one note to an artifact. Errors are reasons to skip it.
fn note_from(
    path: &Path,
    document: &Document,
    report: &mut ImportReport,
) -> std::result::Result<Note, String> {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let artifact_id = document
        .field("id")
        .and_then(artifact_id_in)
        .or_else(|| artifact_id_in(&stem))
        .ok_or_else(|| "The note has no H/E/F artifact ID.".to_owned())?;
    let kind: &'static str = match artifact_id.chars().next() {
        Some('H') => "H",
        Some('E') => "E",
        _ => "F",
    };
    let title = document
        .field("title")
        .map(str::to_owned)
        .or_else(|| document.title.clone())
        .map(|title| strip_artifact_prefix(&title, &artifact_id))
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| artifact_id.clone());
    let title: String = title.chars().take(300).collect();
    let created_at = date(document.field("created")).unwrap_or_else(now);
    let updated_at = date(document.field("last_updated"))
        .or_else(|| date(document.field("completed")))
        .unwrap_or(created_at)
        .max(created_at);
    let raw_status = document.field("status").unwrap_or_default().trim().to_uppercase();

    let (status, payload, parent_hypothesis_id, parent_experiment_id) = match kind {
        "H" => {
            let fields = assemble(
                document,
                &[
                    ("statement", &["Statement"]),
                    ("mechanism", &["Mechanism"]),
                    ("generalization", &["Why This Might Generalize"]),
                    ("shortcut_risks", &["Shortcut Risks", "Risks / What Would Reject It"]),
                    ("test_plan", &["Test Plan", "Falsifiable Predictions"]),
                    ("conclusion", &["Conclusion", "Evidence"]),
                ],
                "conclusion",
            );
            let status = match raw_status.as_str() {
                "" => "PROPOSED".to_owned(),
                "TESTED" | "IN_PROGRESS" => "TESTING".to_owned(),
                other => other.to_owned(),
            };
            let statement = non_empty(&fields["statement"], &title);
            let payload = json!({
                "statement": statement,
                "mechanism": fields["mechanism"],
                "generalization": fields["generalization"],
                "shortcut_risks": fields["shortcut_risks"],
                "test_plan": fields["test_plan"],
                "conclusion": fields["conclusion"],
            });
            (status, payload, None, None)
        }
        "E" => {
            let fields = assemble(
                document,
                &[
                    ("objective", &["Objective"]),
                    ("procedure", &["Procedure", "Setup", "Assets"]),
                    ("success_criteria", &["Expected Outcome", "Success Criteria"]),
                    ("guardrails", &["Guardrails"]),
                    ("results", &["Results", "Progress"]),
                    ("analysis", &["Analysis"]),
                    ("decision", &["Decision", "Decisions"]),
                ],
                "results",
            );
            let status = match raw_status.as_str() {
                "" => "DESIGNED".to_owned(),
                "IN_PROGRESS" => "RUNNING".to_owned(),
                other => other.to_owned(),
            };
            let completed_at = document
                .field("completed")
                .and_then(|value| NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").ok())
                .map(|value| value.to_string());
            let payload = json!({
                "objective": non_empty(&fields["objective"], &title),
                "procedure": fields["procedure"],
                "success_criteria": fields["success_criteria"],
                "guardrails": fields["guardrails"],
                "results": fields["results"],
                "analysis": fields["analysis"],
                "decision": fields["decision"],
                "completed_at": completed_at,
            });
            let hypothesis = document.field("hypothesis").and_then(|v| artifact_reference(v, 'H'));
            (status, payload, hypothesis, None)
        }
        _ => {
            // `impact` may be a level ("HIGH") or a level with an explanation
            // ("CRITICAL - why it matters"); the explanation is kept with the improvement.
            let (impact, explanation) = split_impact(document.field("impact").unwrap_or_default());
            let mut document = document.clone();
            if let Some(explanation) = explanation {
                document.sections.insert(0, ("Impact".to_owned(), explanation));
            }
            let fields = assemble(
                &document,
                &[
                    ("finding", &["Finding"]),
                    ("evidence", &["Evidence"]),
                    ("improvement", &["What Improved For Real", "Impact"]),
                    ("remaining_debt", &["Remaining Debt"]),
                    ("next_move", &["Next Move"]),
                ],
                "finding",
            );
            let impact = impact.unwrap_or_else(|| {
                report.notes.push(format!("{artifact_id}: impact set to MEDIUM."));
                "MEDIUM".to_owned()
            });
            let payload = json!({
                "finding": non_empty(&fields["finding"], &title),
                "evidence": fields["evidence"],
                "improvement": fields["improvement"],
                "remaining_debt": fields["remaining_debt"],
                "next_move": fields["next_move"],
                "impact": impact,
            });
            let hypothesis = document.field("hypothesis").and_then(|v| artifact_reference(v, 'H'));
            let experiment = document.field("experiment").and_then(|v| artifact_reference(v, 'E'));
            ("PUBLISHED".to_owned(), payload, hypothesis, experiment)
        }
    };
    if kind != "F" && !raw_status.is_empty() && raw_status != status {
        report.notes.push(format!("{artifact_id}: status {raw_status} imported as {status}."));
    }
    Ok(Note {
        artifact_id,
        kind,
        title,
        status,
        payload,
        parent_hypothesis_id,
        parent_experiment_id,
        created_at,
        updated_at,
    })
}

/// Fills each field from its sections in document order. Unmapped sections (and any text
/// before the first section) go to `catch_all` under their own headings.
fn assemble(
    document: &Document,
    mapping: &[(&str, &[&str])],
    catch_all: &str,
) -> BTreeMap<String, String> {
    let mut parts: BTreeMap<String, Vec<(String, String)>> =
        mapping.iter().map(|(field, _)| ((*field).to_owned(), Vec::new())).collect();
    if !document.preamble.is_empty() {
        parts
            .entry(catch_all.to_owned())
            .or_default()
            .push((String::new(), document.preamble.clone()));
    }
    for (heading, body) in &document.sections {
        if body.is_empty() || heading.eq_ignore_ascii_case("links") {
            continue;
        }
        let field = mapping
            .iter()
            .find(|(_, headings)| headings.iter().any(|h| h.eq_ignore_ascii_case(heading)))
            .map_or(catch_all, |(field, _)| *field);
        parts.entry(field.to_owned()).or_default().push((heading.clone(), body.clone()));
    }
    parts
        .into_iter()
        .map(|(field, sections)| {
            let primary = mapping
                .iter()
                .find(|(name, _)| *name == field)
                .and_then(|(_, headings)| headings.first());
            let is_primary =
                |heading: &str| primary.is_some_and(|h| h.eq_ignore_ascii_case(heading));
            let text = match sections.as_slice() {
                [] => String::new(),
                // A field's primary section reads as the field; anything else keeps its heading.
                [(heading, body)] if heading.is_empty() || is_primary(heading) => body.clone(),
                many => many
                    .iter()
                    .map(|(heading, body)| {
                        if heading.is_empty() {
                            body.clone()
                        } else {
                            format!("### {heading}\n\n{body}")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            };
            (field, text)
        })
        .collect()
}

/// `"HIGH"` → (HIGH, none); `"CRITICAL - why"` → (CRITICAL, why); `"why"` → (none, why).
fn split_impact(value: &str) -> (Option<String>, Option<String>) {
    let value = value.trim();
    let levels = ["CRITICAL", "HIGH", "MEDIUM", "LOW"];
    let word: String = value.chars().take_while(char::is_ascii_alphabetic).collect();
    let level = levels.iter().find(|level| word.eq_ignore_ascii_case(level));
    match level {
        Some(level) => {
            let rest = value[word.len()..].trim_start_matches([' ', '-', '—', '–', ':']).trim();
            ((*level).to_owned().into(), (!rest.is_empty()).then(|| rest.to_owned()))
        }
        None => (None, (!value.is_empty()).then(|| value.to_owned())),
    }
}

/// `H001`, `"[[E016]]"` or a file name such as `F003-some-title` → the artifact ID.
fn artifact_id_in(text: &str) -> Option<String> {
    let upper = text.trim().trim_matches(|c| matches!(c, '"' | '\'' | '[' | ']')).to_uppercase();
    let kind = upper.chars().next().filter(|c| matches!(c, 'H' | 'E' | 'F'))?;
    let digits: String = upper[1..].chars().take_while(char::is_ascii_digit).collect();
    (digits.len() >= 3).then(|| format!("{kind}{digits}"))
}

fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() { fallback.to_owned() } else { value.to_owned() }
}

fn date(value: Option<&str>) -> Option<Timestamp> {
    let value = value?.trim();
    let day = value.get(..10).unwrap_or(value);
    NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()?.and_hms_opt(0, 0, 0).map(|d| d.and_utc())
}

async fn read_document(path: &Path) -> Result<Option<Document>> {
    match tokio::fs::read_to_string(path).await {
        Ok(text) => Ok(Some(Document::parse(&text))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error(error)),
    }
}

fn io_error(error: std::io::Error) -> LiminaError {
    LiminaError::invariant("The knowledge base could not be read.")
        .with("reason", error.to_string())
}
