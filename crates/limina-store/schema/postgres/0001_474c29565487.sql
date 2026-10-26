-- Alembic revision 474c29565487 (previous: none)

CREATE TABLE challenges (
    id VARCHAR(36) NOT NULL, 
    slug VARCHAR(120) NOT NULL, 
    name VARCHAR(240) NOT NULL, 
    objective TEXT NOT NULL, 
    context TEXT NOT NULL, 
    success_criteria TEXT NOT NULL, 
    status VARCHAR(32) NOT NULL, 
    version INTEGER NOT NULL, 
    created_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    PRIMARY KEY (id), 
    UNIQUE (slug)
);

CREATE TABLE command_receipts (
    command_id VARCHAR(64) NOT NULL, 
    command_type VARCHAR(80) NOT NULL, 
    actor VARCHAR(200) NOT NULL, 
    result JSON NOT NULL, 
    created_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    PRIMARY KEY (command_id)
);

CREATE TABLE project_resources (
    id VARCHAR(36) NOT NULL, 
    challenge_id VARCHAR(36) NOT NULL, 
    name VARCHAR(160) NOT NULL, 
    uri TEXT NOT NULL, 
    kind VARCHAR(40) NOT NULL, 
    credential_env VARCHAR(160), 
    status VARCHAR(24) NOT NULL, 
    created_by VARCHAR(200) NOT NULL, 
    created_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    PRIMARY KEY (id), 
    FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE, 
    CONSTRAINT uq_project_resource_name UNIQUE (challenge_id, name)
);

CREATE INDEX ix_project_resource_challenge ON project_resources (challenge_id, status);

CREATE TABLE artifacts (
    uid VARCHAR(36) NOT NULL, 
    challenge_id VARCHAR(36) NOT NULL, 
    artifact_id VARCHAR(16) NOT NULL, 
    kind VARCHAR(4) NOT NULL, 
    title VARCHAR(300) NOT NULL, 
    status VARCHAR(32) NOT NULL, 
    payload JSON NOT NULL, 
    parent_hypothesis_id VARCHAR(16), 
    parent_experiment_id VARCHAR(16), 
    version INTEGER NOT NULL, 
    created_by VARCHAR(200) NOT NULL, 
    created_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    PRIMARY KEY (uid), 
    FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE, 
    CONSTRAINT uq_artifact_challenge_display_id UNIQUE (challenge_id, artifact_id)
);

CREATE INDEX ix_artifact_challenge_kind ON artifacts (challenge_id, kind);

CREATE TABLE coordinator_states (
    challenge_id VARCHAR(36) NOT NULL, 
    status VARCHAR(32) NOT NULL, 
    current_objective TEXT NOT NULL, 
    next_step TEXT NOT NULL, 
    blocker TEXT NOT NULL, 
    worker_id VARCHAR(200), 
    thread_id VARCHAR(200), 
    inbox_cursor INTEGER NOT NULL, 
    version INTEGER NOT NULL, 
    heartbeat_at TIMESTAMP WITH TIME ZONE, 
    wake_at TIMESTAMP WITH TIME ZONE, 
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    PRIMARY KEY (challenge_id), 
    FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);

CREATE TABLE events (
    sequence SERIAL NOT NULL, 
    id VARCHAR(36) NOT NULL, 
    challenge_id VARCHAR(36) NOT NULL, 
    event_type VARCHAR(80) NOT NULL, 
    actor VARCHAR(200) NOT NULL, 
    artifact_id VARCHAR(16), 
    payload JSON NOT NULL, 
    command_id VARCHAR(64) NOT NULL, 
    created_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    PRIMARY KEY (sequence), 
    FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE, 
    UNIQUE (id)
);

CREATE INDEX ix_event_challenge_sequence ON events (challenge_id, sequence);

CREATE TABLE id_counters (
    challenge_id VARCHAR(36) NOT NULL, 
    kind VARCHAR(4) NOT NULL, 
    next_value INTEGER NOT NULL, 
    PRIMARY KEY (challenge_id, kind), 
    FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);

CREATE TABLE inbox_messages (
    sequence SERIAL NOT NULL, 
    id VARCHAR(36) NOT NULL, 
    challenge_id VARCHAR(36) NOT NULL, 
    kind VARCHAR(32) NOT NULL, 
    body TEXT NOT NULL, 
    actor VARCHAR(200) NOT NULL, 
    status VARCHAR(16) NOT NULL, 
    command_id VARCHAR(64) NOT NULL, 
    created_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    acknowledged_at TIMESTAMP WITH TIME ZONE, 
    PRIMARY KEY (sequence), 
    FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE, 
    UNIQUE (command_id), 
    UNIQUE (id)
);

CREATE INDEX ix_inbox_challenge_status ON inbox_messages (challenge_id, status);

CREATE TABLE work_leases (
    challenge_id VARCHAR(36) NOT NULL, 
    scope VARCHAR(80) NOT NULL, 
    owner VARCHAR(200) NOT NULL, 
    token VARCHAR(36) NOT NULL, 
    expires_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    version INTEGER NOT NULL, 
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    PRIMARY KEY (challenge_id, scope), 
    FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);

CREATE TABLE artifact_revisions (
    id SERIAL NOT NULL, 
    artifact_uid VARCHAR(36) NOT NULL, 
    version INTEGER NOT NULL, 
    status VARCHAR(32) NOT NULL, 
    title VARCHAR(300) NOT NULL, 
    payload JSON NOT NULL, 
    actor VARCHAR(200) NOT NULL, 
    command_id VARCHAR(64) NOT NULL, 
    created_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    PRIMARY KEY (id), 
    FOREIGN KEY(artifact_uid) REFERENCES artifacts (uid) ON DELETE CASCADE, 
    CONSTRAINT uq_artifact_revision_version UNIQUE (artifact_uid, version)
);

CREATE TABLE observations (
    id VARCHAR(36) NOT NULL, 
    challenge_id VARCHAR(36) NOT NULL, 
    experiment_id VARCHAR(16) NOT NULL, 
    body TEXT NOT NULL, 
    evidence_ref TEXT, 
    actor VARCHAR(200) NOT NULL, 
    command_id VARCHAR(64) NOT NULL, 
    created_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    PRIMARY KEY (id), 
    FOREIGN KEY(challenge_id, experiment_id) REFERENCES artifacts (challenge_id, artifact_id) ON DELETE CASCADE, 
    UNIQUE (command_id)
);

CREATE INDEX ix_observation_experiment ON observations (challenge_id, experiment_id);
