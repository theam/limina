//! A deterministic Markdown projection of a project's canonical store: the `kb/` layout Limina
//! agents read (mission, active state, dashboard and one note per H, E and F). The output is
//! byte-identical to the Python runtime's exporter for the same data.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use limina_core::{LiminaError, Result, internal, iso};
use serde_json::Value;

use crate::service::{ArtifactDetail, ArtifactView, ChallengeService, ChallengeView};

/// Exports a project's knowledge base as Markdown files.
#[derive(Debug, Clone)]
pub struct MarkdownExporter {
    service: ChallengeService,
}

impl MarkdownExporter {
    pub fn new(service: ChallengeService) -> Self {
        Self { service }
    }

    /// A focused reader projection. Child collections are intentionally omitted: callers
    /// browse them with a paginated parent query instead of embedding the complete graph.
    pub async fn document(&self, slug: &str, key: &str) -> Result<BTreeMap<String, String>> {
        let key = key.trim().to_uppercase();
        if matches!(key.as_str(), "CHALLENGE" | "ACTIVE") {
            let challenge = self.service.get_challenge(slug).await?;
            let (path, body) = if key == "CHALLENGE" {
                ("mission/CHALLENGE.md", challenge_note(&challenge))
            } else {
                ("ACTIVE.md", active_note(&challenge, &[])?)
            };
            return Ok(BTreeMap::from([(path.to_owned(), body)]));
        }
        let detail = self.service.get_artifact(slug, &key).await?;
        let item = &detail.artifact;
        let (folder, body) = match item.kind.as_str() {
            "H" => ("hypotheses", hypothesis_note(item, &[], &[])),
            "E" => ("experiments", experiment_note(&detail, &[])),
            "F" => {
                let mut parents = Vec::new();
                for id in [&item.hypothesis_id, &item.experiment_id].into_iter().flatten() {
                    parents.push(self.service.get_artifact(slug, id).await?.artifact);
                }
                let by_id = parents.iter().map(|parent| (parent.id.as_str(), parent)).collect();
                ("findings", finding_note(item, &by_id)?)
            }
            _ => return Err(LiminaError::not_found("Knowledge note not found.")),
        };
        let path = format!("research/{folder}/{}-{}.md", item.id, slugify(&item.title));
        Ok(BTreeMap::from([(path, body)]))
    }

    /// The export as `relative path → content`, sorted by path.
    pub async fn snapshot(&self, slug: &str) -> Result<BTreeMap<String, String>> {
        let status = self.service.status(slug).await?;
        let challenge = status.challenge;
        let artifacts = self.service.list_artifacts(slug, None).await?;
        let by_id: HashMap<&str, &ArtifactView> =
            artifacts.iter().map(|item| (item.id.as_str(), item)).collect();
        let mut experiments_by_hypothesis: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut findings_by_hypothesis: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut findings_by_experiment: HashMap<&str, Vec<&str>> = HashMap::new();
        for item in &artifacts {
            if item.kind == "E"
                && let Some(hypothesis) = non_empty(item.hypothesis_id.as_deref())
            {
                experiments_by_hypothesis.entry(hypothesis).or_default().push(&item.id);
            }
            if item.kind == "F" {
                if let Some(hypothesis) = non_empty(item.hypothesis_id.as_deref()) {
                    findings_by_hypothesis.entry(hypothesis).or_default().push(&item.id);
                }
                if let Some(experiment) = non_empty(item.experiment_id.as_deref()) {
                    findings_by_experiment.entry(experiment).or_default().push(&item.id);
                }
            }
        }
        let linked = |map: &HashMap<&str, Vec<&str>>, id: &str| -> Vec<String> {
            map.get(id)
                .map(|ids| ids.iter().map(|id| (*id).to_owned()).collect())
                .unwrap_or_default()
        };

        let mut files = BTreeMap::new();
        files.insert("mission/CHALLENGE.md".to_owned(), challenge_note(&challenge));
        files.insert("ACTIVE.md".to_owned(), active_note(&challenge, &artifacts)?);
        files.insert("DASHBOARD.md".to_owned(), dashboard_note(&challenge, &artifacts));
        for item in &artifacts {
            let filename = format!("{}-{}.md", item.id, slugify(&item.title));
            match item.kind.as_str() {
                "H" => {
                    files.insert(
                        format!("research/hypotheses/{filename}"),
                        hypothesis_note(
                            item,
                            &linked(&experiments_by_hypothesis, &item.id),
                            &linked(&findings_by_hypothesis, &item.id),
                        ),
                    );
                }
                "E" => {
                    let detailed = self.service.get_artifact(slug, &item.id).await?;
                    files.insert(
                        format!("research/experiments/{filename}"),
                        experiment_note(&detailed, &linked(&findings_by_experiment, &item.id)),
                    );
                }
                "F" => {
                    files.insert(
                        format!("research/findings/{filename}"),
                        finding_note(item, &by_id)?,
                    );
                }
                _ => {}
            }
        }
        Ok(files)
    }

    /// Writes the export into `target`, which must be missing or empty so stale artifacts cannot
    /// survive. Returns the written paths in snapshot order.
    pub async fn write(&self, slug: &str, target: &Path) -> Result<Vec<PathBuf>> {
        let snapshot = self.snapshot(slug).await?;
        if tokio::fs::try_exists(target).await.map_err(internal)? {
            let mut entries = tokio::fs::read_dir(target).await.map_err(internal)?;
            if entries.next_entry().await.map_err(internal)?.is_some() {
                return Err(LiminaError::conflict(format!(
                    "Export target '{}' is not empty.",
                    target.display()
                ))
                .with("target", target.display().to_string())
                .with(
                    "suggestion",
                    "Choose an empty directory so stale artifacts cannot survive the export.",
                ));
            }
        }
        let mut written = Vec::with_capacity(snapshot.len());
        for (relative, content) in snapshot {
            let path = target.join(&relative);
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(internal)?;
            }
            tokio::fs::write(&path, content).await.map_err(internal)?;
            written.push(path);
        }
        Ok(written)
    }
}

/// `json.dumps(value, ensure_ascii=False)` for a string: a quoted JSON string that keeps
/// non-ASCII characters and escapes only quotes, backslashes and control characters.
fn yaml(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for c in value.chars() {
        match c {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            '\u{8}' => quoted.push_str("\\b"),
            '\u{c}' => quoted.push_str("\\f"),
            c if u32::from(c) < 0x20 => quoted.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

/// `_yaml` of an optional string: `null` when absent.
fn yaml_opt(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), yaml)
}

/// `_yaml` of any JSON value.
fn yaml_value(value: &Value) -> String {
    match value {
        Value::String(text) => yaml(text),
        other => other.to_string(),
    }
}

/// A file-name slug: lower-case ASCII letters and digits, other runs collapsed to `-`.
fn slugify(value: &str) -> String {
    let mut slug = String::new();
    for c in value.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() { "untitled".to_owned() } else { slug.to_owned() }
}

/// The trimmed text, or a placeholder when it is missing or blank.
fn section(value: Option<&Value>) -> String {
    match value.and_then(Value::as_str).map(crate::service::internal::strip) {
        Some(text) if !text.is_empty() => text.to_owned(),
        _ => "_Not recorded._".to_owned(),
    }
}

fn section_text(value: &str) -> String {
    section(Some(&Value::from(value)))
}

/// Python's `str(value)` for an optional ID inside a wikilink.
fn shown(value: Option<&str>) -> &str {
    value.unwrap_or("None")
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}

/// The `YYYY-MM-DD` prefix of an ISO timestamp.
fn day(value: &limina_core::Timestamp) -> String {
    iso(value).chars().take(10).collect()
}

fn challenge_note(challenge: &ChallengeView) -> String {
    format!(
        "---
aliases: [\"CHALLENGE\"]
type: mission
challenge_id: {id}
challenge_slug: {slug}
---

# Research Mission — {name}

## Objective

{objective}

## Context

{context}

## Success Criteria

{success_criteria}

## Resources & Boundaries

_Managed by the cloud challenge policy._

## Constraints

- Persist durable evidence through typed Limina commands.
- Treat this export as a projection; the challenge service remains canonical.

## Blocked Stop Condition

Stop when the coordinator records a blocker requiring human input.

## Links

- Active State: [[ACTIVE]]
- Dashboard: [[DASHBOARD]]
",
        id = yaml(&challenge.id),
        slug = yaml(&challenge.slug),
        name = challenge.name,
        objective = challenge.objective,
        context = section_text(&challenge.context),
        success_criteria = challenge.success_criteria,
    )
}

fn active_note(challenge: &ChallengeView, artifacts: &[ArtifactView]) -> Result<String> {
    let coordinator = challenge
        .coordinator
        .as_ref()
        .ok_or_else(|| internal(format!("Project {} has no coordinator state", challenge.slug)))?;
    let working: Vec<String> = artifacts
        .iter()
        .filter(|item| {
            (item.kind == "H" && matches!(item.status.as_str(), "PROPOSED" | "TESTING"))
                || (item.kind == "E" && matches!(item.status.as_str(), "DESIGNED" | "RUNNING"))
        })
        .map(|item| format!("- Working Note: [[{}]]", item.id))
        .collect();
    let working_links = if working.is_empty() {
        "- Dashboard: [[DASHBOARD]]".to_owned()
    } else {
        working.join("\n")
    };
    Ok(format!(
        "---
aliases: [\"ACTIVE\"]
type: active-state
coordinator_version: {version}
---

# Active State

## Current Objective

{current_objective}

## Next Step

{next_step}

## Blocker

{blocker}

## Links

- Mission: [[CHALLENGE]]
{working_links}
",
        version = coordinator.version,
        current_objective = coordinator.current_objective,
        next_step = coordinator.next_step,
        blocker = coordinator.blocker,
    ))
}

fn dashboard_note(challenge: &ChallengeView, artifacts: &[ArtifactView]) -> String {
    let lines = |kind: &str| -> String {
        let lines: Vec<String> = artifacts
            .iter()
            .filter(|item| item.kind == kind)
            .map(|item| format!("- [[{}]] — {} ({})", item.id, item.title, item.status))
            .collect();
        if lines.is_empty() { "- None yet.".to_owned() } else { lines.join("\n") }
    };
    format!(
        "---
aliases: [\"DASHBOARD\"]
type: dashboard
challenge_slug: {slug}
---

# {name} — Dashboard

## Entry Points

- [[CHALLENGE]]
- [[ACTIVE]]

## Hypotheses

{hypotheses}

## Experiments

{experiments}

## Findings

{findings}

## Links

- Active State: [[ACTIVE]]
- Mission: [[CHALLENGE]]
",
        slug = yaml(&challenge.slug),
        name = challenge.name,
        hypotheses = lines("H"),
        experiments = lines("E"),
        findings = lines("F"),
    )
}

fn hypothesis_note(item: &ArtifactView, experiments: &[String], findings: &[String]) -> String {
    let payload = &item.payload;
    let mut links =
        vec!["- Mission: [[CHALLENGE]]".to_owned(), "- Active State: [[ACTIVE]]".to_owned()];
    links.extend(experiments.iter().map(|id| format!("- Experiment: [[{id}]]")));
    links.extend(findings.iter().map(|id| format!("- Finding: [[{id}]]")));
    let created = day(&item.created_at);
    let updated = day(&item.updated_at);
    format!(
        "---
id: {id_yaml}
aliases: [{id_yaml}]
type: hypothesis
status: {status_yaml}
created: {created_yaml}
last_updated: {updated_yaml}
tags: []
---

# {id} — {title}

> **Status**: {status}
> **Created**: {created}
> **Last updated**: {updated}

## Statement

{statement}

## Mechanism

{mechanism}

## Why This Might Generalize

{generalization}

## Shortcut Risks

{shortcut_risks}

## Test Plan

{test_plan}

## Evidence

_See linked experiments and findings._

## Conclusion

{conclusion}

## Links

{links}
",
        id_yaml = yaml(&item.id),
        status_yaml = yaml(&item.status),
        created_yaml = yaml(&created),
        updated_yaml = yaml(&updated),
        id = item.id,
        title = item.title,
        status = item.status,
        statement = section(payload.get("statement")),
        mechanism = section(payload.get("mechanism")),
        generalization = section(payload.get("generalization")),
        shortcut_risks = section(payload.get("shortcut_risks")),
        test_plan = section(payload.get("test_plan")),
        conclusion = section(payload.get("conclusion")),
        links = links.join("\n"),
    )
}

fn experiment_note(detail: &ArtifactDetail, findings: &[String]) -> String {
    let item = &detail.artifact;
    let payload = &item.payload;
    let observations = detail.observations.as_deref().unwrap_or_default();
    let progress = if observations.is_empty() {
        "- [ ] No observations recorded.".to_owned()
    } else {
        observations
            .iter()
            .map(|entry| {
                let evidence = non_empty(entry.evidence_ref.as_deref())
                    .map(|reference| format!(" — `{reference}`"))
                    .unwrap_or_default();
                format!("- [x] ({}) {}{evidence}", day(&entry.created_at), entry.body)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let hypothesis_id = item.hypothesis_id.as_deref();
    let mut links = vec![
        "- Mission: [[CHALLENGE]]".to_owned(),
        "- Active State: [[ACTIVE]]".to_owned(),
        format!("- Parent Hypothesis: [[{}]]", shown(hypothesis_id)),
    ];
    links.extend(findings.iter().map(|id| format!("- Finding: [[{id}]]")));
    // `payload.get("completed_at") or ""`
    let (completed, completed_yaml) =
        match payload.get("completed_at").filter(|value| !is_falsy(value)) {
            None => (String::new(), yaml("")),
            Some(Value::String(text)) => (text.clone(), yaml(text)),
            Some(other) => (other.to_string(), yaml_value(other)),
        };
    let created = day(&item.created_at);
    format!(
        "---
id: {id_yaml}
aliases: [{id_yaml}]
type: experiment
status: {status_yaml}
hypothesis: {hypothesis_yaml}
created: {created_yaml}
completed: {completed_yaml}
tags: []
---

# {id} — {title}

> **Status**: {status}
> **Hypothesis**: [[{hypothesis}]]
> **Created**: {created}
> **Completed**: {completed}

## Objective

{objective}

## Setup

{guardrails}

## Procedure

{procedure}

## Expected Outcome

{success_criteria}

## Progress

{progress}

## Results

{results}

## Analysis

{analysis}

## Decision

{decision}

## Links

{links}
",
        id_yaml = yaml(&item.id),
        status_yaml = yaml(&item.status),
        hypothesis_yaml = yaml_opt(hypothesis_id),
        created_yaml = yaml(&created),
        id = item.id,
        title = item.title,
        status = item.status,
        hypothesis = shown(hypothesis_id),
        objective = section(payload.get("objective")),
        guardrails = section(payload.get("guardrails")),
        procedure = section(payload.get("procedure")),
        success_criteria = section(payload.get("success_criteria")),
        results = section(payload.get("results")),
        analysis = section(payload.get("analysis")),
        decision = section(payload.get("decision")),
        links = links.join("\n"),
    )
}

fn finding_note(item: &ArtifactView, by_id: &HashMap<&str, &ArtifactView>) -> Result<String> {
    let payload = &item.payload;
    let resolved = |id: Option<&str>| id.is_some_and(|id| by_id.contains_key(id));
    let (hypothesis_id, experiment_id) =
        (item.hypothesis_id.as_deref(), item.experiment_id.as_deref());
    if !resolved(hypothesis_id) || !resolved(experiment_id) {
        return Err(internal(format!("Finding {} has unresolved parents", item.id)));
    }
    let impact = payload
        .get("impact")
        .ok_or_else(|| internal(format!("Finding {} has no impact", item.id)))?;
    let impact_text = match impact {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let created = day(&item.created_at);
    Ok(format!(
        "---
id: {id_yaml}
aliases: [{id_yaml}]
type: finding
hypothesis: {hypothesis_yaml}
experiment: {experiment_yaml}
impact: {impact_yaml}
created: {created_yaml}
tags: []
---

# {id} — {title}

> **Created**: {created}
> **Hypothesis**: [[{hypothesis}]]
> **Experiment**: [[{experiment}]]
> **Impact**: {impact}

## Finding

{finding}

## Evidence

{evidence}

## What Improved For Real

{improvement}

## Remaining Debt

{remaining_debt}

## Next Move

{next_move}

## Links

- Mission: [[CHALLENGE]]
- Active State: [[ACTIVE]]
- Parent Hypothesis: [[{hypothesis}]]
- Parent Experiment: [[{experiment}]]
",
        id_yaml = yaml(&item.id),
        hypothesis_yaml = yaml_opt(hypothesis_id),
        experiment_yaml = yaml_opt(experiment_id),
        impact_yaml = yaml_value(impact),
        created_yaml = yaml(&created),
        id = item.id,
        title = item.title,
        hypothesis = shown(hypothesis_id),
        experiment = shown(experiment_id),
        impact = impact_text,
        finding = section(payload.get("finding")),
        evidence = section(payload.get("evidence")),
        improvement = section(payload.get("improvement")),
        remaining_debt = section(payload.get("remaining_debt")),
        next_move = section(payload.get("next_move")),
    ))
}

/// Python truthiness for JSON values.
fn is_falsy(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(value) => !value,
        Value::Number(number) => number.as_f64() == Some(0.0),
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_strings_match_python_json_dumps() {
        assert_eq!(yaml("plain"), "\"plain\"");
        assert_eq!(yaml("é \"q\" \\ /"), "\"é \\\"q\\\" \\\\ /\"");
        assert_eq!(yaml("a\nb\tc\u{1}\u{8}\u{c}\r\u{7f}"), "\"a\\nb\\tc\\u0001\\b\\f\\r\u{7f}\"");
        assert_eq!(yaml_opt(None), "null");
    }

    #[test]
    fn slugs_collapse_everything_but_ascii_letters_and_digits() {
        assert_eq!(slugify("Reranking improves offline NDCG"), "reranking-improves-offline-ndcg");
        assert_eq!(slugify("  --Été 2026: Δ++ "), "t-2026");
        assert_eq!(slugify("!!!"), "untitled");
        assert_eq!(slugify("KELVIN \u{212a}"), "kelvin-k");
    }

    #[test]
    fn sections_fall_back_when_blank() {
        assert_eq!(section(None), "_Not recorded._");
        assert_eq!(section(Some(&Value::from(" \n "))), "_Not recorded._");
        assert_eq!(section(Some(&Value::from(" kept \n"))), "kept");
        assert_eq!(section(Some(&Value::Null)), "_Not recorded._");
    }
}
