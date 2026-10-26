-- Alembic revision 7e4a19b8d2c6 (previous: c0d3c1a2b7e9)

CREATE TABLE project_members (
    challenge_id VARCHAR(36) NOT NULL, 
    subject VARCHAR(300) NOT NULL, 
    role VARCHAR(16) NOT NULL, 
    display_name VARCHAR(240) NOT NULL, 
    email VARCHAR(320), 
    created_by VARCHAR(300) NOT NULL, 
    created_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    PRIMARY KEY (challenge_id, subject), 
    FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);

CREATE INDEX ix_project_member_subject ON project_members (subject, role);

CREATE TABLE live_tickets (
    token_hash VARCHAR(64) NOT NULL, 
    challenge_id VARCHAR(36) NOT NULL, 
    subject VARCHAR(300) NOT NULL, 
    display_name VARCHAR(240) NOT NULL, 
    role VARCHAR(16) NOT NULL, 
    instance_admin BOOLEAN NOT NULL, 
    expires_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    used_at TIMESTAMP WITH TIME ZONE, 
    created_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    PRIMARY KEY (token_hash), 
    FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);

CREATE INDEX ix_live_ticket_expiry ON live_tickets (expires_at, used_at);

CREATE TABLE project_sources (
    id VARCHAR(36) NOT NULL, 
    challenge_id VARCHAR(36) NOT NULL, 
    name VARCHAR(200) NOT NULL, 
    source_type VARCHAR(24) NOT NULL, 
    uri TEXT NOT NULL, 
    media_type VARCHAR(200), 
    metadata_json JSON NOT NULL, 
    status VARCHAR(24) NOT NULL, 
    created_by VARCHAR(200) NOT NULL, 
    created_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    PRIMARY KEY (id), 
    FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE, 
    CONSTRAINT uq_project_source_name UNIQUE (challenge_id, name)
);

CREATE INDEX ix_project_source_challenge ON project_sources (challenge_id, status);

CREATE TABLE knowledge_relations (
    id VARCHAR(36) NOT NULL, 
    challenge_id VARCHAR(36) NOT NULL, 
    source_artifact_id VARCHAR(16) NOT NULL, 
    target_artifact_id VARCHAR(16) NOT NULL, 
    relation_type VARCHAR(80) NOT NULL, 
    description TEXT NOT NULL, 
    created_by VARCHAR(200) NOT NULL, 
    created_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    PRIMARY KEY (id), 
    FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE, 
    FOREIGN KEY(challenge_id, source_artifact_id) REFERENCES artifacts (challenge_id, artifact_id) ON DELETE CASCADE, 
    FOREIGN KEY(challenge_id, target_artifact_id) REFERENCES artifacts (challenge_id, artifact_id) ON DELETE CASCADE, 
    CONSTRAINT uq_knowledge_relation UNIQUE (challenge_id, source_artifact_id, target_artifact_id, relation_type)
);

CREATE INDEX ix_relation_source ON knowledge_relations (challenge_id, source_artifact_id);

CREATE INDEX ix_relation_target ON knowledge_relations (challenge_id, target_artifact_id);

CREATE TABLE artifact_comments (
    id VARCHAR(36) NOT NULL, 
    challenge_id VARCHAR(36) NOT NULL, 
    artifact_id VARCHAR(16) NOT NULL, 
    body TEXT NOT NULL, 
    actor VARCHAR(200) NOT NULL, 
    command_id VARCHAR(64) NOT NULL, 
    created_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    PRIMARY KEY (id), 
    FOREIGN KEY(challenge_id, artifact_id) REFERENCES artifacts (challenge_id, artifact_id) ON DELETE CASCADE, 
    UNIQUE (command_id)
);

CREATE INDEX ix_artifact_comment ON artifact_comments (challenge_id, artifact_id, created_at);

CREATE TABLE artifact_tags (
    challenge_id VARCHAR(36) NOT NULL, 
    artifact_id VARCHAR(16) NOT NULL, 
    tag VARCHAR(80) NOT NULL, 
    created_by VARCHAR(200) NOT NULL, 
    created_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    PRIMARY KEY (challenge_id, artifact_id, tag), 
    FOREIGN KEY(challenge_id, artifact_id) REFERENCES artifacts (challenge_id, artifact_id) ON DELETE CASCADE
);

CREATE INDEX ix_artifact_tag_lookup ON artifact_tags (challenge_id, tag);

CREATE TABLE saved_knowledge_views (
    id VARCHAR(36) NOT NULL, 
    challenge_id VARCHAR(36) NOT NULL, 
    name VARCHAR(160) NOT NULL, 
    query JSON NOT NULL, 
    created_by VARCHAR(200) NOT NULL, 
    created_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    PRIMARY KEY (id), 
    FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE, 
    CONSTRAINT uq_saved_view_name UNIQUE (challenge_id, name)
);

CREATE INDEX ix_saved_view_challenge ON saved_knowledge_views (challenge_id, updated_at);

CREATE TABLE runtime_runs (
    id VARCHAR(36) NOT NULL, 
    challenge_id VARCHAR(36) NOT NULL, 
    runtime_engine VARCHAR(32) NOT NULL, 
    model VARCHAR(160), 
    status VARCHAR(24) NOT NULL, 
    summary TEXT NOT NULL, 
    error_code VARCHAR(80), 
    error_message TEXT, 
    turn_id VARCHAR(200), 
    input_tokens BIGINT, 
    output_tokens BIGINT, 
    cached_input_tokens BIGINT, 
    cost_microusd BIGINT, 
    tool_calls INTEGER NOT NULL, 
    retry_count INTEGER NOT NULL, 
    started_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    completed_at TIMESTAMP WITH TIME ZONE, 
    duration_ms BIGINT, 
    PRIMARY KEY (id), 
    FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);

CREATE INDEX ix_runtime_run_challenge_started ON runtime_runs (challenge_id, started_at);

CREATE INDEX ix_runtime_run_status ON runtime_runs (challenge_id, status);

CREATE INDEX ix_artifact_search_fts ON artifacts USING GIN (
                to_tsvector(
                    'simple',
                    coalesce(title, '') || ' ' || coalesce(payload::text, '')
                )
            );
