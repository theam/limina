//! Rules and storage steps shared by the challenge, project and research commands.

use chrono::Duration;
use limina_core::{LiminaError, Result, Timestamp, new_uuid, now};
use limina_store::models::{Artifact, WorkLease};
use limina_store::{Tx, args};
use serde_json::Value;

use crate::environment::is_reserved_resource_name;

/// The largest variable or secret value, in UTF-8 bytes.
pub const RESOURCE_VALUE_LIMIT: usize = 32_768;
/// The longest guidance message, in characters.
pub const MESSAGE_LIMIT: usize = 32_768;

/// Python's `str.strip()`: Unicode whitespace plus the ASCII separators U+001C–U+001F, which
/// Python also treats as whitespace. Stored values must be trimmed exactly like the Python runtime
/// trimmed them.
pub fn strip(value: &str) -> &str {
    value.trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}

/// Rejects empty or whitespace-only text: `"<Label> cannot be empty."` with a `field` detail.
pub fn require_text(label: &str, value: &str) -> Result<()> {
    if strip(value).is_empty() {
        return Err(LiminaError::invariant(format!("{} cannot be empty.", capitalize(label)))
            .with("field", label));
    }
    Ok(())
}

/// Python's `str.capitalize()`: the first character upper-cased, the rest lower-cased.
fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars.flat_map(char::to_lowercase)).collect())
        .unwrap_or_default()
}

pub fn validate_lease_ttl(ttl_seconds: i64) -> Result<()> {
    if !(30..=86_400).contains(&ttl_seconds) {
        return Err(LiminaError::invariant("Lease TTL must be between 30 seconds and 24 hours."));
    }
    Ok(())
}

/// `$first, $first+1, …` for `count` bound values (an `IN (…)` list).
pub fn placeholders(first: usize, count: usize) -> String {
    (first..first + count).map(|index| format!("${index}")).collect::<Vec<_>>().join(", ")
}

/// An artifact of a project by display ID (case-insensitive), optionally of one kind.
pub async fn find_artifact(
    tx: &mut Tx,
    challenge_id: &str,
    artifact_id: &str,
    kind: Option<&str>,
) -> Result<Artifact> {
    let artifact: Option<Artifact> = tx
        .fetch_optional(
            "SELECT * FROM artifacts WHERE challenge_id = $1 AND artifact_id = $2",
            args![challenge_id, artifact_id.to_uppercase()],
        )
        .await?;
    match artifact {
        Some(artifact) if kind.is_none_or(|kind| artifact.kind == kind) => Ok(artifact),
        _ => Err(LiminaError::not_found(format!("Artifact '{artifact_id}' does not exist."))
            .with("artifact_id", artifact_id)),
    }
}

/// The next display ID of a kind (`H001`, `E012`, …), allocated atomically per project.
pub async fn allocate_artifact_id(tx: &mut Tx, challenge_id: &str, kind: &str) -> Result<String> {
    let allocated: i32 = tx
        .scalar(
            "INSERT INTO id_counters (challenge_id, kind, next_value) VALUES ($1, $2, 2) \
             ON CONFLICT(challenge_id, kind) DO UPDATE SET \
             next_value = id_counters.next_value + 1 \
             RETURNING next_value - 1",
            args![challenge_id, kind],
        )
        .await?;
    Ok(format!("{kind}{allocated:03}"))
}

/// A new artifact at version 1.
pub struct NewArtifact<'a> {
    pub challenge_id: &'a str,
    pub kind: &'a str,
    pub title: &'a str,
    pub status: &'a str,
    pub payload: Value,
    pub parent_hypothesis_id: Option<&'a str>,
    pub parent_experiment_id: Option<&'a str>,
    pub created_by: &'a str,
}

/// Allocates the display ID, inserts the artifact and records its first revision.
pub async fn create_artifact(
    tx: &mut Tx,
    new: NewArtifact<'_>,
    command_id: &str,
) -> Result<Artifact> {
    let artifact_id = allocate_artifact_id(tx, new.challenge_id, new.kind).await?;
    let created_at = now();
    let artifact: Artifact = tx
        .fetch_one(
            "INSERT INTO artifacts (uid, challenge_id, artifact_id, kind, title, status, payload, \
             parent_hypothesis_id, parent_experiment_id, version, created_by, created_at, \
             updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 1, $10, $11, $11) RETURNING *",
            args![
                new_uuid(),
                new.challenge_id,
                artifact_id,
                new.kind,
                new.title,
                new.status,
                new.payload,
                new.parent_hypothesis_id,
                new.parent_experiment_id,
                new.created_by,
                created_at
            ],
        )
        .await?;
    record_revision(tx, &artifact, new.created_by, command_id).await?;
    Ok(artifact)
}

/// Snapshots an artifact's current version into its revision history.
pub async fn record_revision(
    tx: &mut Tx,
    artifact: &Artifact,
    actor: &str,
    command_id: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO artifact_revisions (artifact_uid, version, status, title, payload, actor, \
         command_id, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        args![
            &artifact.uid,
            artifact.version,
            &artifact.status,
            &artifact.title,
            &artifact.payload.0,
            actor,
            command_id,
            now()
        ],
    )
    .await?;
    Ok(())
}

/// Optimistic update: sets status and payload only if the artifact is still at
/// `expected_version`, bumps the version and records the revision.
#[allow(clippy::too_many_arguments)]
pub async fn cas_artifact(
    tx: &mut Tx,
    artifact: &Artifact,
    expected_version: i32,
    status: &str,
    payload: Value,
    actor: &str,
    command_id: &str,
) -> Result<Artifact> {
    let updated: Option<Artifact> = tx
        .fetch_optional(
            "UPDATE artifacts SET status = $1, payload = $2, version = version + 1, \
             updated_at = $3 WHERE uid = $4 AND version = $5 RETURNING *",
            args![status, payload, now(), &artifact.uid, expected_version],
        )
        .await?;
    let Some(updated) = updated else {
        let current: Option<i32> = tx
            .scalar_optional("SELECT version FROM artifacts WHERE uid = $1", args![&artifact.uid])
            .await?;
        let shown = current.map_or_else(|| "None".to_owned(), |version| version.to_string());
        return Err(LiminaError::conflict(format!(
            "{} changed from v{expected_version} to v{shown}.",
            artifact.artifact_id
        ))
        .with("artifact_id", artifact.artifact_id.as_str())
        .with("expected_version", expected_version)
        .with("current_version", current));
    };
    record_revision(tx, &updated, actor, command_id).await?;
    Ok(updated)
}

/// The unexpired lease on `scope`, which `actor` must own.
pub async fn require_active_lease(
    tx: &mut Tx,
    challenge_id: &str,
    scope: &str,
    actor: &str,
) -> Result<WorkLease> {
    let lease = find_lease(tx, challenge_id, scope).await?.ok_or_else(|| {
        LiminaError::lease_conflict(format!("{scope} must be claimed before it can run."))
            .with("artifact_id", scope)
    })?;
    let expires_at = limina_core::iso(&lease.expires_at);
    if lease.expires_at <= now() {
        return Err(LiminaError::lease_conflict(format!(
            "The lease for {scope} expired at {expires_at}."
        ))
        .with("artifact_id", scope)
        .with("expires_at", expires_at));
    }
    if lease.owner != actor {
        return Err(LiminaError::lease_conflict(format!(
            "{scope} is claimed by {}; current actor is {actor}.",
            lease.owner
        ))
        .with("artifact_id", scope)
        .with("owner", lease.owner.as_str())
        .with("actor", actor)
        .with("expires_at", expires_at));
    }
    Ok(lease)
}

pub async fn find_lease(tx: &mut Tx, challenge_id: &str, scope: &str) -> Result<Option<WorkLease>> {
    Ok(tx
        .fetch_optional(
            "SELECT * FROM work_leases WHERE challenge_id = $1 AND scope = $2",
            args![challenge_id, scope],
        )
        .await?)
}

/// Takes or renews the lease on `scope` for `owner`. Succeeds when the scope is free, its lease
/// expired, or `owner` already holds it; the upsert makes concurrent claims race safely.
pub async fn acquire_lease(
    tx: &mut Tx,
    challenge_id: &str,
    scope: &str,
    owner: &str,
    ttl_seconds: i64,
) -> Result<WorkLease> {
    let now: Timestamp = now();
    let expires_at = now + Duration::seconds(ttl_seconds);
    let lease: Option<WorkLease> = tx
        .fetch_optional(
            "INSERT INTO work_leases (challenge_id, scope, owner, token, expires_at, version, \
             updated_at) VALUES ($1, $2, $3, $4, $5, 1, $6) \
             ON CONFLICT(challenge_id, scope) DO UPDATE SET \
             owner = excluded.owner, \
             token = excluded.token, \
             expires_at = excluded.expires_at, \
             version = work_leases.version + 1, \
             updated_at = excluded.updated_at \
             WHERE work_leases.expires_at <= $6 OR work_leases.owner = $3 \
             RETURNING *",
            args![challenge_id, scope, owner, new_uuid(), expires_at, now],
        )
        .await?;
    if let Some(lease) = lease {
        return Ok(lease);
    }
    let existing = find_lease(tx, challenge_id, scope).await?;
    let current_owner = existing.as_ref().map_or("another worker", |lease| lease.owner.as_str());
    Err(LiminaError::lease_conflict(format!("{scope} is already claimed by {current_owner}."))
        .with("scope", scope)
        .with("owner", existing.as_ref().map(|lease| lease.owner.clone()))
        .with("expires_at", existing.as_ref().map(|lease| limina_core::iso(&lease.expires_at))))
}

/// A resource name as stored: trimmed, upper-cased, a valid environment variable name and not
/// reserved by the managed runtime.
pub fn resource_name(value: &str) -> Result<String> {
    let name = strip(value).to_uppercase();
    let mut chars = name.chars();
    let valid = chars.next().is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return Err(LiminaError::invariant(
            "Resource names must be valid environment variable names.",
        )
        .with("name", value)
        .with("suggestion", "Use letters, numbers, and underscores, beginning with a letter."));
    }
    if is_reserved_resource_name(&name) {
        return Err(LiminaError::invariant(format!(
            "Resource name '{name}' is reserved by the managed runtime."
        ))
        .with("name", name.as_str()));
    }
    Ok(name)
}

/// A non-empty resource value within the size limit. Values are stored as given (not trimmed).
pub fn resource_value(resource_type: &str, value: &str) -> Result<()> {
    require_text(&format!("resource {resource_type}"), value)?;
    if value.len() > RESOURCE_VALUE_LIMIT {
        return Err(LiminaError::invariant(format!(
            "Resource {resource_type} exceeds the {RESOURCE_VALUE_LIMIT}-byte limit."
        ))
        .with("resource_type", resource_type)
        .with("limit", RESOURCE_VALUE_LIMIT));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_rules_match_python() {
        assert_eq!(strip("\u{1f} a b \u{3000}\n"), "a b");
        assert_eq!(capitalize("current objective"), "Current objective");
        assert_eq!(capitalize("source URI"), "Source uri");
        let error = require_text("next step", " \u{1c} ").unwrap_err();
        assert_eq!(error.message, "Next step cannot be empty.");
        assert_eq!(error.details["field"], "next step");
        assert_eq!(placeholders(3, 2), "$3, $4");
    }

    #[test]
    fn resource_names_are_environment_names() {
        assert_eq!(resource_name(" source_url ").unwrap(), "SOURCE_URL");
        assert_eq!(resource_name("_x1").unwrap(), "_X1");
        assert_eq!(resource_name("ﬀ_X").unwrap(), "FF_X", "upper-cased like Python");
        let invalid = resource_name("1BAD").unwrap_err();
        assert_eq!(invalid.details["name"], "1BAD");
        assert_eq!(
            invalid.details["suggestion"],
            "Use letters, numbers, and underscores, beginning with a letter."
        );
        let reserved = resource_name("path").unwrap_err();
        assert_eq!(reserved.message, "Resource name 'PATH' is reserved by the managed runtime.");
        assert!(resource_value("variable", &"x".repeat(RESOURCE_VALUE_LIMIT)).is_ok());
        let large =
            resource_value("secret", &"é".repeat(RESOURCE_VALUE_LIMIT / 2 + 1)).unwrap_err();
        assert_eq!(large.message, "Resource secret exceeds the 32768-byte limit.");
        assert_eq!(large.details["limit"], 32_768);
    }
}
