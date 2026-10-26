//! Knowledge collaboration: search with filters and cursors, revisions, the knowledge graph,
//! relations, comments, tags, saved views and project sources, as a second `impl` block of
//! [`CollaborationService`].

use std::collections::HashMap;

use limina_core::time::{serde_iso, serde_iso_opt};
use limina_core::{LiminaError, Result, Timestamp, internal, new_uuid, now};
use limina_store::db::Backend;
use limina_store::models::{
    Artifact, ArtifactComment, ArtifactRevision, KnowledgeRelation, ProjectSource,
    SavedKnowledgeView,
};
use limina_store::{Arg, Tx, args};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::collaboration::paging::{self, DEFAULT_PAGE_LIMIT};
use crate::collaboration::validation::{
    SAVED_VIEW_QUERY_LIMIT, normalize_tag, py_strip, python_json_len, required,
    validate_source_metadata, validate_source_uri,
};
use crate::collaboration::{CollaborationService, knowledge_artifact, project, record};
use crate::common::record_event;

/// The longest relation type, in characters.
pub const RELATION_TYPE_LIMIT: usize = 80;
/// The longest comment, in characters.
pub const COMMENT_LIMIT: usize = 32_768;
/// The filters a saved view may store.
pub const SAVED_VIEW_FILTERS: [&str; 5] = ["query", "kind", "status", "tag", "parent"];
/// Kinds of project source.
pub const SOURCE_TYPES: [&str; 3] = ["URL", "CONNECTOR", "UPLOAD"];

/// The full-text document PostgreSQL indexes (`ix_artifact_search_fts`); written exactly like the
/// index expression so the planner can use it.
const SEARCH_DOCUMENT: &str =
    "to_tsvector('simple', coalesce(title, '') || ' ' || coalesce(payload::text, ''))";

/// A knowledge artifact with its tags (`_artifact_dict`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeArtifact {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub status: String,
    pub content: Value,
    pub hypothesis_id: Option<String>,
    pub experiment_id: Option<String>,
    #[serde(with = "serde_iso")]
    pub created_at: Timestamp,
    #[serde(with = "serde_iso")]
    pub updated_at: Timestamp,
    pub tags: Vec<String>,
}

impl KnowledgeArtifact {
    fn from_row(row: Artifact, tags: Vec<String>) -> Self {
        Self {
            id: row.artifact_id,
            kind: row.kind,
            title: row.title,
            status: row.status,
            content: row.payload.0,
            hypothesis_id: row.parent_hypothesis_id,
            experiment_id: row.parent_experiment_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
            tags,
        }
    }
}

/// Filters and paging for [`CollaborationService::query_knowledge`]. Empty strings mean "no
/// filter", like absent ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KnowledgeQuery {
    /// Text to search for in titles and content.
    pub query: Option<String>,
    pub kind: Option<String>,
    pub status: Option<String>,
    pub tag: Option<String>,
    /// Limit results to evidence belonging to this hypothesis or experiment.
    pub parent: Option<String>,
    pub cursor: Option<String>,
    pub limit: i64,
}

impl Default for KnowledgeQuery {
    fn default() -> Self {
        Self {
            query: None,
            kind: None,
            status: None,
            tag: None,
            parent: None,
            cursor: None,
            limit: DEFAULT_PAGE_LIMIT,
        }
    }
}

/// One page of knowledge: `{"items", "next_cursor", "total", "search_backend"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgePage {
    pub items: Vec<KnowledgeArtifact>,
    pub next_cursor: Option<String>,
    pub total: i64,
    /// `postgresql-fts` or `portable-substring`.
    pub search_backend: String,
}

/// One stored version of an artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Revision {
    pub version: i32,
    pub status: String,
    pub title: String,
    pub content: Value,
    pub actor: String,
    #[serde(with = "serde_iso")]
    pub created_at: Timestamp,
}

impl From<ArtifactRevision> for Revision {
    fn from(row: ArtifactRevision) -> Self {
        Self {
            version: row.version,
            status: row.status,
            title: row.title,
            content: row.payload.0,
            actor: row.actor,
            created_at: row.created_at,
        }
    }
}

/// An edge of the knowledge graph: an explicit relation, or one derived from an artifact's parent
/// (`TESTED_BY` from its hypothesis, `FOUND` from its experiment).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relation {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    #[serde(rename = "type")]
    pub relation_type: String,
    pub description: String,
    pub derived: bool,
    pub created_by: String,
    #[serde(with = "serde_iso_opt")]
    pub created_at: Option<Timestamp>,
}

impl Relation {
    fn derived(source_id: &str, target_id: &str, relation_type: &str) -> Self {
        Self {
            id: format!("derived:{source_id}:{relation_type}:{target_id}"),
            source_id: source_id.to_owned(),
            target_id: target_id.to_owned(),
            relation_type: relation_type.to_owned(),
            description: String::new(),
            derived: true,
            created_by: "Limina".to_owned(),
            created_at: None,
        }
    }
}

impl From<KnowledgeRelation> for Relation {
    fn from(row: KnowledgeRelation) -> Self {
        Self {
            id: row.id,
            source_id: row.source_artifact_id,
            target_id: row.target_artifact_id,
            relation_type: row.relation_type,
            description: row.description,
            derived: false,
            created_by: row.created_by,
            created_at: Some(row.created_at),
        }
    }
}

/// A new explicit relation (`POST /knowledge/relations`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewRelation {
    pub source_id: String,
    pub target_id: String,
    #[serde(rename = "type")]
    pub relation_type: String,
    #[serde(default)]
    pub description: String,
}

/// Every artifact and every edge between them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeGraph {
    pub nodes: Vec<KnowledgeArtifact>,
    pub edges: Vec<Relation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub artifact_id: String,
    pub body: String,
    pub actor: String,
    #[serde(with = "serde_iso")]
    pub created_at: Timestamp,
}

impl From<ArtifactComment> for Comment {
    fn from(row: ArtifactComment) -> Self {
        Self {
            id: row.id,
            artifact_id: row.artifact_id,
            body: row.body,
            actor: row.actor,
            created_at: row.created_at,
        }
    }
}

/// A named knowledge query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedView {
    pub id: String,
    pub name: String,
    pub query: Map<String, Value>,
    pub created_by: String,
    #[serde(with = "serde_iso")]
    pub created_at: Timestamp,
    #[serde(with = "serde_iso")]
    pub updated_at: Timestamp,
}

impl TryFrom<SavedKnowledgeView> for SavedView {
    type Error = LiminaError;

    fn try_from(row: SavedKnowledgeView) -> Result<Self> {
        Ok(Self {
            id: row.id,
            name: row.name,
            query: object(row.query.0)?,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

/// A source of material for the project: a URL, a connector reference or an uploaded file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub source_type: String,
    pub uri: String,
    pub media_type: Option<String>,
    pub metadata: Map<String, Value>,
    pub status: String,
    #[serde(with = "serde_iso")]
    pub created_at: Timestamp,
    #[serde(with = "serde_iso")]
    pub updated_at: Timestamp,
}

impl TryFrom<ProjectSource> for Source {
    type Error = LiminaError;

    fn try_from(row: ProjectSource) -> Result<Self> {
        Ok(Self {
            id: row.id,
            name: row.name,
            source_type: row.source_type,
            uri: row.uri,
            media_type: row.media_type,
            metadata: object(row.metadata_json.0)?,
            status: row.status,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

/// A source to add or replace (`PUT /sources`), keyed by name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceInput {
    pub name: String,
    #[serde(rename = "type")]
    pub source_type: String,
    pub uri: String,
    #[serde(default)]
    pub media_type: Option<String>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

impl CollaborationService {
    /// Searches and filters knowledge, most recently updated first (best match first on
    /// PostgreSQL full-text search).
    pub async fn query_knowledge(
        &self,
        slug: &str,
        request: &KnowledgeQuery,
    ) -> Result<KnowledgePage> {
        let offset = paging::knowledge_page_offset(request.cursor.as_deref())?;
        let limit = paging::clamp_limit(request.limit);
        let mut tx = self.db.begin().await?;
        let challenge = project(&mut tx, slug).await?;
        let postgres = tx.backend() == Backend::Postgres;
        let mut args = args![&challenge.id];
        let mut filters = vec!["challenge_id = $1".to_owned()];
        let mut rank = String::new();
        if let Some(kind) = present(&request.kind) {
            args.push(kind.to_uppercase().into());
            filters.push(format!("kind = ${}", args.len()));
        }
        if let Some(status) = present(&request.status) {
            args.push(status.to_uppercase().into());
            filters.push(format!("status = ${}", args.len()));
        }
        if let Some(parent) = present(&request.parent) {
            args.push(parent.to_uppercase().into());
            let n = args.len();
            filters.push(format!("(parent_hypothesis_id = ${n} OR parent_experiment_id = ${n})"));
        }
        if let Some(tag) = present(&request.tag) {
            args.push(normalize_tag(tag)?.into());
            filters.push(format!(
                "artifact_id IN (SELECT artifact_id FROM artifact_tags \
                 WHERE challenge_id = $1 AND tag = ${})",
                args.len()
            ));
        }
        if let Some(text) = request.query.as_deref().map(py_strip).filter(|text| !text.is_empty()) {
            let identifier = text.to_uppercase();
            let prefix = identifier.trim_end_matches(|c: char| c.is_ascii_digit());
            if matches!(prefix, "H" | "E" | "F") && identifier.len() > prefix.len() {
                args.push(identifier.into());
                filters.push(format!("artifact_id = ${}", args.len()));
            } else if postgres {
                args.push(text.into());
                let query = format!("websearch_to_tsquery('simple', ${})", args.len());
                filters.push(format!("{SEARCH_DOCUMENT} @@ {query}"));
                rank = format!("ts_rank_cd({SEARCH_DOCUMENT}, {query}) DESC, ");
            } else {
                args.push(like_contains(&text.to_lowercase()).into());
                let pattern = format!("'%' || ${} || '%' ESCAPE '/'", args.len());
                filters.push(format!(
                    "(lower(title) LIKE {pattern} OR lower(CAST(payload AS TEXT)) LIKE {pattern})"
                ));
            }
        }
        let filter = filters.join(" AND ");
        let total: i64 = tx
            .scalar(
                &format!("SELECT CAST(COUNT(*) AS BIGINT) FROM artifacts WHERE {filter}"),
                args.clone(),
            )
            .await?;
        let next = args.len() + 1;
        args.push(limit.into());
        args.push(offset.into());
        let rows: Vec<Artifact> = tx
            .fetch_all(
                &format!(
                    "SELECT * FROM artifacts WHERE {filter} ORDER BY {rank}updated_at DESC, \
                     artifact_id LIMIT ${next} OFFSET ${}",
                    next + 1
                ),
                args,
            )
            .await?;
        let ids: Vec<&str> = rows.iter().map(|row| row.artifact_id.as_str()).collect();
        let mut tags = tags_by_artifact(&mut tx, &challenge.id, Some(&ids)).await?;
        tx.commit().await?;
        let next_cursor = paging::next_cursor(offset, rows.len(), total);
        let items = rows
            .into_iter()
            .map(|row| {
                let tags = tags.remove(&row.artifact_id).unwrap_or_default();
                KnowledgeArtifact::from_row(row, tags)
            })
            .collect();
        Ok(KnowledgePage {
            items,
            next_cursor,
            total,
            search_backend: if postgres { "postgresql-fts" } else { "portable-substring" }
                .to_owned(),
        })
    }

    /// Every stored version of an artifact, newest first.
    pub async fn revisions(&self, slug: &str, artifact_id: &str) -> Result<Vec<Revision>> {
        let mut tx = self.db.begin().await?;
        let challenge = project(&mut tx, slug).await?;
        let artifact = knowledge_artifact(&mut tx, &challenge.id, artifact_id).await?;
        let rows: Vec<ArtifactRevision> = tx
            .fetch_all(
                "SELECT * FROM artifact_revisions WHERE artifact_uid = $1 ORDER BY version DESC",
                args![&artifact.uid],
            )
            .await?;
        tx.commit().await?;
        Ok(rows.into_iter().map(Revision::from).collect())
    }

    /// One bounded history page, strictly older than the supplied version.
    pub async fn revisions_before(
        &self,
        slug: &str,
        artifact_id: &str,
        before: i32,
    ) -> Result<Vec<Revision>> {
        let mut tx = self.db.begin().await?;
        let challenge = project(&mut tx, slug).await?;
        let artifact = knowledge_artifact(&mut tx, &challenge.id, artifact_id).await?;
        let rows: Vec<ArtifactRevision> = tx.fetch_all(
            "SELECT * FROM artifact_revisions WHERE artifact_uid = $1 AND version < $2 ORDER BY version DESC LIMIT 26",
            args![&artifact.uid, before],
        ).await?;
        tx.commit().await?;
        Ok(rows.into_iter().map(Revision::from).collect())
    }

    /// Every artifact with its tags, explicit relations (oldest first) and the edges implied by
    /// artifact parents.
    pub async fn graph(&self, slug: &str) -> Result<KnowledgeGraph> {
        let mut tx = self.db.begin().await?;
        let challenge = project(&mut tx, slug).await?;
        let artifacts: Vec<Artifact> = tx
            .fetch_all(
                "SELECT * FROM artifacts WHERE challenge_id = $1 ORDER BY artifact_id",
                args![&challenge.id],
            )
            .await?;
        let explicit: Vec<KnowledgeRelation> = tx
            .fetch_all(
                "SELECT * FROM knowledge_relations WHERE challenge_id = $1 ORDER BY created_at, id",
                args![&challenge.id],
            )
            .await?;
        // Every tag of the project belongs to one of these artifacts.
        let mut tags = tags_by_artifact(&mut tx, &challenge.id, None).await?;
        tx.commit().await?;
        let mut edges: Vec<Relation> = explicit.into_iter().map(Relation::from).collect();
        for artifact in &artifacts {
            if let Some(hypothesis) =
                artifact.parent_hypothesis_id.as_deref().filter(|id| !id.is_empty())
            {
                edges.push(Relation::derived(hypothesis, &artifact.artifact_id, "TESTED_BY"));
            }
            if let Some(experiment) =
                artifact.parent_experiment_id.as_deref().filter(|id| !id.is_empty())
            {
                edges.push(Relation::derived(experiment, &artifact.artifact_id, "FOUND"));
            }
        }
        let nodes = artifacts
            .into_iter()
            .map(|row| {
                let tags = tags.remove(&row.artifact_id).unwrap_or_default();
                KnowledgeArtifact::from_row(row, tags)
            })
            .collect();
        Ok(KnowledgeGraph { nodes, edges })
    }

    /// Links two artifacts. The relation type is upper-cased with spaces as underscores.
    pub async fn create_relation(
        &self,
        slug: &str,
        relation: &NewRelation,
        actor: &str,
    ) -> Result<Relation> {
        let relation_type = py_strip(&relation.relation_type).to_uppercase().replace(' ', "_");
        if relation_type.is_empty() || relation_type.chars().count() > RELATION_TYPE_LIMIT {
            return Err(LiminaError::invariant("Relation type must be 1 to 80 characters."));
        }
        if relation.source_id == relation.target_id {
            return Err(LiminaError::invariant("A knowledge relation cannot point to itself."));
        }
        let mut tx = self.db.begin_write().await?;
        let challenge = project(&mut tx, slug).await?;
        knowledge_artifact(&mut tx, &challenge.id, &relation.source_id).await?;
        knowledge_artifact(&mut tx, &challenge.id, &relation.target_id).await?;
        let inserted = tx
            .fetch_one::<KnowledgeRelation>(
                "INSERT INTO knowledge_relations (id, challenge_id, source_artifact_id, \
                 target_artifact_id, relation_type, description, created_by, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING *",
                args![
                    new_uuid(),
                    &challenge.id,
                    &relation.source_id,
                    &relation.target_id,
                    relation_type,
                    py_strip(&relation.description),
                    actor,
                    now()
                ],
            )
            .await;
        let row = match inserted {
            Ok(row) => row,
            Err(error) if error.is_unique_violation() || error.is_foreign_key_violation() => {
                return Err(LiminaError::conflict("That knowledge relation already exists."));
            }
            Err(error) => return Err(error.into()),
        };
        record(
            &mut tx,
            &challenge.id,
            "knowledge.relation_created",
            actor,
            json!({
                "relation_id": &row.id,
                "source_id": &relation.source_id,
                "target_id": &relation.target_id,
            }),
            Some(&relation.source_id),
        )
        .await?;
        tx.commit().await?;
        Ok(row.into())
    }

    pub async fn delete_relation(
        &self,
        slug: &str,
        relation_id: &str,
        actor: &str,
    ) -> Result<Relation> {
        let mut tx = self.db.begin_write().await?;
        let challenge = project(&mut tx, slug).await?;
        let row: Option<KnowledgeRelation> = tx
            .fetch_optional(
                "SELECT * FROM knowledge_relations WHERE challenge_id = $1 AND id = $2",
                args![&challenge.id, relation_id],
            )
            .await?;
        let Some(row) = row else {
            return Err(LiminaError::not_found("Knowledge relation does not exist."));
        };
        tx.execute("DELETE FROM knowledge_relations WHERE id = $1", args![&row.id]).await?;
        record(
            &mut tx,
            &challenge.id,
            "knowledge.relation_removed",
            actor,
            json!({"relation_id": relation_id}),
            Some(&row.source_artifact_id),
        )
        .await?;
        tx.commit().await?;
        Ok(row.into())
    }

    /// An artifact's comments, oldest first.
    pub async fn comments(&self, slug: &str, artifact_id: &str) -> Result<Vec<Comment>> {
        let mut tx = self.db.begin().await?;
        let challenge = project(&mut tx, slug).await?;
        let artifact = knowledge_artifact(&mut tx, &challenge.id, artifact_id).await?;
        let rows: Vec<ArtifactComment> = tx
            .fetch_all(
                "SELECT * FROM artifact_comments WHERE challenge_id = $1 AND artifact_id = $2 \
                 ORDER BY created_at",
                args![&challenge.id, &artifact.artifact_id],
            )
            .await?;
        tx.commit().await?;
        Ok(rows.into_iter().map(Comment::from).collect())
    }

    /// Comments on an artifact, idempotent on `command_id`: a retry returns the stored comment,
    /// and a command ID used for another project, artifact or actor is a conflict.
    pub async fn add_comment(
        &self,
        slug: &str,
        artifact_id: &str,
        body: &str,
        actor: &str,
        command_id: &str,
    ) -> Result<Comment> {
        let body = required("comment", body)?;
        if body.chars().count() > COMMENT_LIMIT {
            return Err(LiminaError::invariant("Comment must be at most 32768 characters."));
        }
        match self.write_comment(slug, artifact_id, &body, actor, command_id).await {
            Err(error) if error.is_constraint_violation() => {
                // A concurrent retry of the same command won the race; replay its comment.
                let mut tx = self.db.begin().await?;
                let challenge = project(&mut tx, slug).await?;
                let artifact = knowledge_artifact(&mut tx, &challenge.id, artifact_id).await?;
                let existing = comment_by_command(&mut tx, command_id).await?;
                tx.commit().await?;
                match existing {
                    Some(existing) => replay_comment(
                        existing,
                        &challenge.id,
                        &artifact.artifact_id,
                        actor,
                        command_id,
                    ),
                    None => Err(LiminaError::conflict(
                        "The comment conflicted with another concurrent write.",
                    )
                    .with("command_id", command_id)),
                }
            }
            outcome => outcome,
        }
    }

    async fn write_comment(
        &self,
        slug: &str,
        artifact_id: &str,
        body: &str,
        actor: &str,
        command_id: &str,
    ) -> Result<Comment> {
        let mut tx = self.db.begin_write().await?;
        let challenge = project(&mut tx, slug).await?;
        let artifact = knowledge_artifact(&mut tx, &challenge.id, artifact_id).await?;
        if let Some(existing) = comment_by_command(&mut tx, command_id).await? {
            return replay_comment(
                existing,
                &challenge.id,
                &artifact.artifact_id,
                actor,
                command_id,
            );
        }
        let row: ArtifactComment = tx
            .fetch_one(
                "INSERT INTO artifact_comments (id, challenge_id, artifact_id, body, actor, \
                 command_id, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING *",
                args![
                    new_uuid(),
                    &challenge.id,
                    &artifact.artifact_id,
                    body,
                    actor,
                    command_id,
                    now()
                ],
            )
            .await?;
        record_event(
            &mut tx,
            &challenge.id,
            "knowledge.comment_added",
            actor,
            command_id,
            json!({"comment_id": &row.id}),
            Some(&artifact.artifact_id),
        )
        .await?;
        tx.commit().await?;
        Ok(row.into())
    }

    /// An artifact's tags, in order.
    pub async fn tags(&self, slug: &str, artifact_id: &str) -> Result<Vec<String>> {
        let mut tx = self.db.begin().await?;
        let challenge = project(&mut tx, slug).await?;
        knowledge_artifact(&mut tx, &challenge.id, artifact_id).await?;
        let tags = artifact_tags(&mut tx, &challenge.id, &artifact_id.to_uppercase()).await?;
        tx.commit().await?;
        Ok(tags)
    }

    /// Tags an artifact (adding an existing tag changes nothing) and returns its tags.
    pub async fn add_tag(
        &self,
        slug: &str,
        artifact_id: &str,
        tag: &str,
        actor: &str,
    ) -> Result<Vec<String>> {
        let tag = normalize_tag(tag)?;
        let mut tx = self.db.begin_write().await?;
        let challenge = project(&mut tx, slug).await?;
        let artifact = knowledge_artifact(&mut tx, &challenge.id, artifact_id).await?;
        let added = tx
            .execute(
                "INSERT INTO artifact_tags (challenge_id, artifact_id, tag, created_by, created_at) \
                 VALUES ($1, $2, $3, $4, $5) ON CONFLICT (challenge_id, artifact_id, tag) DO NOTHING",
                args![&challenge.id, &artifact.artifact_id, &tag, actor, now()],
            )
            .await?;
        if added > 0 {
            record(
                &mut tx,
                &challenge.id,
                "knowledge.tag_added",
                actor,
                json!({"tag": &tag}),
                Some(&artifact.artifact_id),
            )
            .await?;
        }
        let tags = artifact_tags(&mut tx, &challenge.id, &artifact.artifact_id).await?;
        tx.commit().await?;
        Ok(tags)
    }

    /// Removes a tag from an artifact and returns its remaining tags.
    pub async fn remove_tag(
        &self,
        slug: &str,
        artifact_id: &str,
        tag: &str,
        actor: &str,
    ) -> Result<Vec<String>> {
        let tag = normalize_tag(tag)?;
        let mut tx = self.db.begin_write().await?;
        let challenge = project(&mut tx, slug).await?;
        let artifact = knowledge_artifact(&mut tx, &challenge.id, artifact_id).await?;
        let removed = tx
            .execute(
                "DELETE FROM artifact_tags WHERE challenge_id = $1 AND artifact_id = $2 AND tag = $3",
                args![&challenge.id, &artifact.artifact_id, &tag],
            )
            .await?;
        if removed == 0 {
            return Err(LiminaError::not_found("Knowledge tag does not exist.").with("tag", tag));
        }
        record(
            &mut tx,
            &challenge.id,
            "knowledge.tag_removed",
            actor,
            json!({"tag": &tag}),
            Some(&artifact.artifact_id),
        )
        .await?;
        let tags = artifact_tags(&mut tx, &challenge.id, &artifact.artifact_id).await?;
        tx.commit().await?;
        Ok(tags)
    }

    /// Saved views, by name.
    pub async fn saved_views(&self, slug: &str) -> Result<Vec<SavedView>> {
        let mut tx = self.db.begin().await?;
        let challenge = project(&mut tx, slug).await?;
        let rows: Vec<SavedKnowledgeView> = tx
            .fetch_all(
                "SELECT * FROM saved_knowledge_views WHERE challenge_id = $1 ORDER BY name",
                args![&challenge.id],
            )
            .await?;
        tx.commit().await?;
        rows.into_iter().map(SavedView::try_from).collect()
    }

    /// Saves a named query, replacing the query of a view with the same name.
    pub async fn save_view(
        &self,
        slug: &str,
        name: &str,
        query: &Map<String, Value>,
        actor: &str,
    ) -> Result<SavedView> {
        let name = required("view name", name)?;
        if query.keys().any(|key| !SAVED_VIEW_FILTERS.contains(&key.as_str())) {
            return Err(LiminaError::invariant("Saved view query contains unsupported filters."));
        }
        let query = Value::Object(query.clone());
        if python_json_len(&query) > SAVED_VIEW_QUERY_LIMIT {
            return Err(LiminaError::invariant("Saved view query is too large."));
        }
        let mut tx = self.db.begin_write().await?;
        let challenge = project(&mut tx, slug).await?;
        let row: SavedKnowledgeView = tx
            .fetch_one(
                "INSERT INTO saved_knowledge_views (id, challenge_id, name, \"query\", created_by, \
                 created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $6) \
                 ON CONFLICT (challenge_id, name) DO UPDATE SET \"query\" = excluded.\"query\", \
                 updated_at = excluded.updated_at RETURNING *",
                args![new_uuid(), &challenge.id, &name, query, actor, now()],
            )
            .await?;
        record(
            &mut tx,
            &challenge.id,
            "knowledge.view_saved",
            actor,
            json!({"view_id": &row.id, "name": &name}),
            None,
        )
        .await?;
        tx.commit().await?;
        row.try_into()
    }

    pub async fn delete_view(&self, slug: &str, view_id: &str, actor: &str) -> Result<SavedView> {
        let mut tx = self.db.begin_write().await?;
        let challenge = project(&mut tx, slug).await?;
        let row: Option<SavedKnowledgeView> = tx
            .fetch_optional(
                "SELECT * FROM saved_knowledge_views WHERE challenge_id = $1 AND id = $2",
                args![&challenge.id, view_id],
            )
            .await?;
        let Some(row) = row else {
            return Err(LiminaError::not_found("Saved knowledge view does not exist."));
        };
        tx.execute("DELETE FROM saved_knowledge_views WHERE id = $1", args![&row.id]).await?;
        record(
            &mut tx,
            &challenge.id,
            "knowledge.view_removed",
            actor,
            json!({"view_id": view_id}),
            None,
        )
        .await?;
        tx.commit().await?;
        row.try_into()
    }

    /// Active project sources, by name.
    pub async fn sources(&self, slug: &str) -> Result<Vec<Source>> {
        let mut tx = self.db.begin().await?;
        let challenge = project(&mut tx, slug).await?;
        let rows: Vec<ProjectSource> = tx
            .fetch_all(
                "SELECT * FROM project_sources WHERE challenge_id = $1 AND status = 'ACTIVE' \
                 ORDER BY name",
                args![&challenge.id],
            )
            .await?;
        tx.commit().await?;
        rows.into_iter().map(Source::try_from).collect()
    }

    /// Adds a source, or replaces (and reactivates) the source with the same name. Sources never
    /// carry credentials: those live in write-only project secrets.
    pub async fn set_source(
        &self,
        slug: &str,
        source: &SourceInput,
        actor: &str,
    ) -> Result<Source> {
        let source_type = source.source_type.to_uppercase();
        if !SOURCE_TYPES.contains(&source_type.as_str()) {
            return Err(LiminaError::invariant("Source type must be URL, CONNECTOR, or UPLOAD."));
        }
        let name = required("source name", &source.name)?;
        let uri = required("source URI", &source.uri)?;
        let media_type = source.media_type.as_deref();
        if name.chars().count() > 200
            || uri.chars().count() > 4096
            || media_type.is_some_and(|media| media.chars().count() > 200)
        {
            return Err(LiminaError::invariant(
                "Source name, URI, or media type exceeds its size limit.",
            ));
        }
        if source_type != "UPLOAD" {
            validate_source_uri(&uri, source_type == "URL")?;
        }
        validate_source_metadata(&source.metadata)?;
        let mut tx = self.db.begin_write().await?;
        let challenge = project(&mut tx, slug).await?;
        let row: ProjectSource = tx
            .fetch_one(
                "INSERT INTO project_sources (id, challenge_id, name, source_type, uri, media_type, \
                 metadata_json, status, created_by, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, 'ACTIVE', $8, $9, $9) \
                 ON CONFLICT (challenge_id, name) DO UPDATE SET source_type = excluded.source_type, \
                 uri = excluded.uri, media_type = excluded.media_type, \
                 metadata_json = excluded.metadata_json, status = 'ACTIVE', \
                 updated_at = excluded.updated_at RETURNING *",
                args![
                    new_uuid(),
                    &challenge.id,
                    &name,
                    &source_type,
                    &uri,
                    media_type,
                    Value::Object(source.metadata.clone()),
                    actor,
                    now()
                ],
            )
            .await?;
        record(
            &mut tx,
            &challenge.id,
            "source.configured",
            actor,
            json!({"source_id": &row.id, "name": &row.name, "type": &source_type}),
            None,
        )
        .await?;
        tx.commit().await?;
        row.try_into()
    }

    /// Deactivates a source; it stays in the database as `REMOVED`.
    pub async fn remove_source(&self, slug: &str, source_id: &str, actor: &str) -> Result<Source> {
        let mut tx = self.db.begin_write().await?;
        let challenge = project(&mut tx, slug).await?;
        let row: Option<ProjectSource> = tx
            .fetch_optional(
                "UPDATE project_sources SET status = 'REMOVED', updated_at = $1 \
                 WHERE challenge_id = $2 AND id = $3 AND status = 'ACTIVE' RETURNING *",
                args![now(), &challenge.id, source_id],
            )
            .await?;
        let Some(row) = row else {
            return Err(LiminaError::not_found("Active project source does not exist."));
        };
        record(
            &mut tx,
            &challenge.id,
            "source.removed",
            actor,
            json!({"source_id": &row.id, "name": &row.name}),
            None,
        )
        .await?;
        tx.commit().await?;
        row.try_into()
    }
}

/// A non-empty filter value.
fn present(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|value| !value.is_empty())
}

/// A `LIKE` pattern body matching `text` literally (SQLAlchemy's `contains(autoescape=True)`).
fn like_contains(text: &str) -> String {
    text.replace('/', "//").replace('%', "/%").replace('_', "/_")
}

fn object(value: Value) -> Result<Map<String, Value>> {
    match value {
        Value::Object(map) => Ok(map),
        other => Err(internal(format!("expected a JSON object, found {other}"))),
    }
}

/// Tags per artifact, each list in tag order. `None` reads every tag of the project.
async fn tags_by_artifact(
    tx: &mut Tx,
    challenge_id: &str,
    artifact_ids: Option<&[&str]>,
) -> Result<HashMap<String, Vec<String>>> {
    let rows: Vec<(String, String)> = match artifact_ids {
        Some([]) => Vec::new(),
        Some(ids) => {
            let placeholders: Vec<String> =
                (2..ids.len() + 2).map(|index| format!("${index}")).collect();
            let mut args = args![challenge_id];
            args.extend(ids.iter().map(|id| Arg::from(*id)));
            tx.fetch_all(
                &format!(
                    "SELECT artifact_id, tag FROM artifact_tags WHERE challenge_id = $1 \
                     AND artifact_id IN ({}) ORDER BY tag",
                    placeholders.join(", ")
                ),
                args,
            )
            .await?
        }
        None => {
            tx.fetch_all(
                "SELECT artifact_id, tag FROM artifact_tags WHERE challenge_id = $1 ORDER BY tag",
                args![challenge_id],
            )
            .await?
        }
    };
    let mut tags: HashMap<String, Vec<String>> = HashMap::new();
    for (artifact_id, tag) in rows {
        tags.entry(artifact_id).or_default().push(tag);
    }
    Ok(tags)
}

async fn artifact_tags(tx: &mut Tx, challenge_id: &str, artifact_id: &str) -> Result<Vec<String>> {
    let rows: Vec<(String,)> = tx
        .fetch_all(
            "SELECT tag FROM artifact_tags WHERE challenge_id = $1 AND artifact_id = $2 ORDER BY tag",
            args![challenge_id, artifact_id],
        )
        .await?;
    Ok(rows.into_iter().map(|(tag,)| tag).collect())
}

async fn comment_by_command(tx: &mut Tx, command_id: &str) -> Result<Option<ArtifactComment>> {
    Ok(tx
        .fetch_optional("SELECT * FROM artifact_comments WHERE command_id = $1", args![command_id])
        .await?)
}

/// The stored comment for a replayed command, unless the command ID belongs to another comment.
fn replay_comment(
    existing: ArtifactComment,
    challenge_id: &str,
    artifact_id: &str,
    actor: &str,
    command_id: &str,
) -> Result<Comment> {
    if existing.challenge_id != challenge_id
        || existing.artifact_id != artifact_id
        || existing.actor != actor
    {
        return Err(LiminaError::conflict(
            "The idempotency key was already used for another comment.",
        )
        .with("command_id", command_id));
    }
    Ok(existing.into())
}
