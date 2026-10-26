-- SQLite schema generated from the runtime models. Applied idempotently on startup.
CREATE TABLE IF NOT EXISTS challenges (
	id VARCHAR(36) NOT NULL, 
	slug VARCHAR(120) NOT NULL, 
	name VARCHAR(240) NOT NULL, 
	objective TEXT NOT NULL, 
	context TEXT NOT NULL, 
	success_criteria TEXT NOT NULL, 
	runtime_engine VARCHAR(32) NOT NULL,
	model VARCHAR(200),
	reasoning_effort VARCHAR(64),
	status VARCHAR(32) NOT NULL, 
	version INTEGER NOT NULL, 
	created_at DATETIME NOT NULL, 
	updated_at DATETIME NOT NULL, 
	PRIMARY KEY (id), 
	UNIQUE (slug)
);

CREATE TABLE IF NOT EXISTS command_receipts (
	command_id VARCHAR(64) NOT NULL, 
	command_type VARCHAR(80) NOT NULL, 
	actor VARCHAR(200) NOT NULL, 
	result JSON NOT NULL, 
	created_at DATETIME NOT NULL, 
	PRIMARY KEY (command_id)
);

CREATE TABLE IF NOT EXISTS artifacts (
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
	created_at DATETIME NOT NULL, 
	updated_at DATETIME NOT NULL, 
	PRIMARY KEY (uid), 
	CONSTRAINT uq_artifact_challenge_display_id UNIQUE (challenge_id, artifact_id), 
	FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS ix_artifact_challenge_kind ON artifacts (challenge_id, kind);

CREATE TABLE IF NOT EXISTS coordinator_states (
	challenge_id VARCHAR(36) NOT NULL, 
	status VARCHAR(32) NOT NULL, 
	current_objective TEXT NOT NULL, 
	next_step TEXT NOT NULL, 
	blocker TEXT NOT NULL, 
	worker_id VARCHAR(200), 
	continuation_id VARCHAR(200), 
	inbox_cursor INTEGER NOT NULL, 
	version INTEGER NOT NULL, 
	heartbeat_at DATETIME, 
	wake_at DATETIME, 
	updated_at DATETIME NOT NULL, 
	PRIMARY KEY (challenge_id), 
	FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS events (
	sequence INTEGER NOT NULL, 
	id VARCHAR(36) NOT NULL, 
	challenge_id VARCHAR(36) NOT NULL, 
	event_type VARCHAR(80) NOT NULL, 
	actor VARCHAR(200) NOT NULL, 
	artifact_id VARCHAR(16), 
	payload JSON NOT NULL, 
	command_id VARCHAR(64) NOT NULL, 
	created_at DATETIME NOT NULL, 
	PRIMARY KEY (sequence), 
	UNIQUE (id), 
	FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS ix_event_challenge_sequence ON events (challenge_id, sequence);

CREATE TABLE IF NOT EXISTS id_counters (
	challenge_id VARCHAR(36) NOT NULL, 
	kind VARCHAR(4) NOT NULL, 
	next_value INTEGER NOT NULL, 
	PRIMARY KEY (challenge_id, kind), 
	FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS inbox_messages (
	sequence INTEGER NOT NULL, 
	id VARCHAR(36) NOT NULL, 
	challenge_id VARCHAR(36) NOT NULL, 
	kind VARCHAR(32) NOT NULL, 
	body TEXT NOT NULL, 
	actor VARCHAR(200) NOT NULL, 
	status VARCHAR(16) NOT NULL, 
	command_id VARCHAR(64) NOT NULL, 
	created_at DATETIME NOT NULL, 
	acknowledged_at DATETIME, 
	PRIMARY KEY (sequence), 
	UNIQUE (id), 
	FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE, 
	UNIQUE (command_id)
);
CREATE INDEX IF NOT EXISTS ix_inbox_challenge_status ON inbox_messages (challenge_id, status);

CREATE TABLE IF NOT EXISTS live_tickets (
	token_hash VARCHAR(64) NOT NULL, 
	challenge_id VARCHAR(36) NOT NULL, 
	subject VARCHAR(300) NOT NULL, 
	display_name VARCHAR(240) NOT NULL, 
	role VARCHAR(16) NOT NULL, 
	instance_admin BOOLEAN NOT NULL, 
	expires_at DATETIME NOT NULL, 
	used_at DATETIME, 
	created_at DATETIME NOT NULL, 
	project_admin BOOLEAN DEFAULT 0 NOT NULL,
	PRIMARY KEY (token_hash), 
	FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS ix_live_ticket_expiry ON live_tickets (expires_at, used_at);

CREATE TABLE IF NOT EXISTS project_engine_accounts (
	challenge_id VARCHAR(36) NOT NULL, 
	engine VARCHAR(32) NOT NULL, 
	method VARCHAR(32) NOT NULL, 
	label VARCHAR(320), 
	secret_ciphertext TEXT, 
	connected_by VARCHAR(200) NOT NULL, 
	connected_by_subject VARCHAR(300) NOT NULL, 
	created_at DATETIME NOT NULL, 
	updated_at DATETIME NOT NULL, 
	PRIMARY KEY (challenge_id, engine), 
	FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS project_invitations (
	challenge_id VARCHAR(36) NOT NULL,
	email VARCHAR(320) NOT NULL,
	role VARCHAR(16) NOT NULL,
	organization_id VARCHAR(300),
	created_by VARCHAR(300) NOT NULL,
	created_at DATETIME NOT NULL,
	updated_at DATETIME NOT NULL,
	PRIMARY KEY (challenge_id, email),
	FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS ix_project_invitation_email ON project_invitations (email);

CREATE TABLE IF NOT EXISTS project_members (
	challenge_id VARCHAR(36) NOT NULL, 
	subject VARCHAR(300) NOT NULL, 
	role VARCHAR(16) NOT NULL, 
	display_name VARCHAR(240) NOT NULL, 
	email VARCHAR(320), 
	created_by VARCHAR(300) NOT NULL, 
	created_at DATETIME NOT NULL, 
	updated_at DATETIME NOT NULL, 
	PRIMARY KEY (challenge_id, subject), 
	FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS ix_project_member_subject ON project_members (subject, role);

CREATE TABLE IF NOT EXISTS project_resources (
	id VARCHAR(36) NOT NULL, 
	challenge_id VARCHAR(36) NOT NULL, 
	name VARCHAR(160) NOT NULL, 
	resource_type VARCHAR(16) NOT NULL, 
	value TEXT, 
	secret_ciphertext TEXT, 
	status VARCHAR(24) NOT NULL, 
	created_by VARCHAR(200) NOT NULL, 
	created_at DATETIME NOT NULL, 
	updated_at DATETIME NOT NULL, 
	PRIMARY KEY (id), 
	CONSTRAINT uq_project_resource_name UNIQUE (challenge_id, name), 
	FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS ix_project_resource_challenge ON project_resources (challenge_id, status);

CREATE TABLE IF NOT EXISTS project_sources (
	id VARCHAR(36) NOT NULL, 
	challenge_id VARCHAR(36) NOT NULL, 
	name VARCHAR(200) NOT NULL, 
	source_type VARCHAR(24) NOT NULL, 
	uri TEXT NOT NULL, 
	media_type VARCHAR(200), 
	metadata_json JSON NOT NULL, 
	status VARCHAR(24) NOT NULL, 
	created_by VARCHAR(200) NOT NULL, 
	created_at DATETIME NOT NULL, 
	updated_at DATETIME NOT NULL, 
	PRIMARY KEY (id), 
	CONSTRAINT uq_project_source_name UNIQUE (challenge_id, name), 
	FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS ix_project_source_challenge ON project_sources (challenge_id, status);

CREATE TABLE IF NOT EXISTS runtime_runs (
	id VARCHAR(36) NOT NULL, 
	challenge_id VARCHAR(36) NOT NULL, 
	runtime_engine VARCHAR(32) NOT NULL,
	model VARCHAR(200),
	status VARCHAR(24) NOT NULL, 
	summary TEXT NOT NULL, 
	error_code VARCHAR(80), 
	error_message TEXT, 
	turn_id VARCHAR(200), 
	input_tokens BIGINT, 
	output_tokens BIGINT, 
	cached_input_tokens BIGINT, 
	reasoning_output_tokens BIGINT, 
	total_tokens BIGINT, 
	cost_microusd BIGINT, 
	usage_source VARCHAR(40), 
	cost_source VARCHAR(40), 
	tool_calls INTEGER NOT NULL, 
	retry_count INTEGER NOT NULL, 
	started_at DATETIME NOT NULL, 
	completed_at DATETIME, 
	duration_ms BIGINT, 
	PRIMARY KEY (id), 
	FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS ix_runtime_run_challenge_started ON runtime_runs (challenge_id, started_at);
CREATE INDEX IF NOT EXISTS ix_runtime_run_status ON runtime_runs (challenge_id, status);

CREATE TABLE IF NOT EXISTS saved_knowledge_views (
	id VARCHAR(36) NOT NULL, 
	challenge_id VARCHAR(36) NOT NULL, 
	name VARCHAR(160) NOT NULL, 
	"query" JSON NOT NULL, 
	created_by VARCHAR(200) NOT NULL, 
	created_at DATETIME NOT NULL, 
	updated_at DATETIME NOT NULL, 
	PRIMARY KEY (id), 
	CONSTRAINT uq_saved_view_name UNIQUE (challenge_id, name), 
	FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS ix_saved_view_challenge ON saved_knowledge_views (challenge_id, updated_at);

CREATE TABLE IF NOT EXISTS work_leases (
	challenge_id VARCHAR(36) NOT NULL, 
	scope VARCHAR(80) NOT NULL, 
	owner VARCHAR(200) NOT NULL, 
	token VARCHAR(36) NOT NULL, 
	expires_at DATETIME NOT NULL, 
	version INTEGER NOT NULL, 
	updated_at DATETIME NOT NULL, 
	PRIMARY KEY (challenge_id, scope), 
	FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS artifact_comments (
	id VARCHAR(36) NOT NULL, 
	challenge_id VARCHAR(36) NOT NULL, 
	artifact_id VARCHAR(16) NOT NULL, 
	body TEXT NOT NULL, 
	actor VARCHAR(200) NOT NULL, 
	command_id VARCHAR(64) NOT NULL, 
	created_at DATETIME NOT NULL, 
	PRIMARY KEY (id), 
	FOREIGN KEY(challenge_id, artifact_id) REFERENCES artifacts (challenge_id, artifact_id) ON DELETE CASCADE, 
	UNIQUE (command_id)
);
CREATE INDEX IF NOT EXISTS ix_artifact_comment ON artifact_comments (challenge_id, artifact_id, created_at);

CREATE TABLE IF NOT EXISTS artifact_revisions (
	id INTEGER NOT NULL, 
	artifact_uid VARCHAR(36) NOT NULL, 
	version INTEGER NOT NULL, 
	status VARCHAR(32) NOT NULL, 
	title VARCHAR(300) NOT NULL, 
	payload JSON NOT NULL, 
	actor VARCHAR(200) NOT NULL, 
	command_id VARCHAR(64) NOT NULL, 
	created_at DATETIME NOT NULL, 
	PRIMARY KEY (id), 
	CONSTRAINT uq_artifact_revision_version UNIQUE (artifact_uid, version), 
	FOREIGN KEY(artifact_uid) REFERENCES artifacts (uid) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS artifact_tags (
	challenge_id VARCHAR(36) NOT NULL, 
	artifact_id VARCHAR(16) NOT NULL, 
	tag VARCHAR(80) NOT NULL, 
	created_by VARCHAR(200) NOT NULL, 
	created_at DATETIME NOT NULL, 
	PRIMARY KEY (challenge_id, artifact_id, tag), 
	FOREIGN KEY(challenge_id, artifact_id) REFERENCES artifacts (challenge_id, artifact_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS ix_artifact_tag_lookup ON artifact_tags (challenge_id, tag);

CREATE TABLE IF NOT EXISTS knowledge_relations (
	id VARCHAR(36) NOT NULL, 
	challenge_id VARCHAR(36) NOT NULL, 
	source_artifact_id VARCHAR(16) NOT NULL, 
	target_artifact_id VARCHAR(16) NOT NULL, 
	relation_type VARCHAR(80) NOT NULL, 
	description TEXT NOT NULL, 
	created_by VARCHAR(200) NOT NULL, 
	created_at DATETIME NOT NULL, 
	PRIMARY KEY (id), 
	FOREIGN KEY(challenge_id, source_artifact_id) REFERENCES artifacts (challenge_id, artifact_id) ON DELETE CASCADE, 
	FOREIGN KEY(challenge_id, target_artifact_id) REFERENCES artifacts (challenge_id, artifact_id) ON DELETE CASCADE, 
	CONSTRAINT uq_knowledge_relation UNIQUE (challenge_id, source_artifact_id, target_artifact_id, relation_type), 
	FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS ix_relation_source ON knowledge_relations (challenge_id, source_artifact_id);
CREATE INDEX IF NOT EXISTS ix_relation_target ON knowledge_relations (challenge_id, target_artifact_id);

CREATE TABLE IF NOT EXISTS observations (
	id VARCHAR(36) NOT NULL, 
	challenge_id VARCHAR(36) NOT NULL, 
	experiment_id VARCHAR(16) NOT NULL, 
	body TEXT NOT NULL, 
	evidence_ref TEXT, 
	actor VARCHAR(200) NOT NULL, 
	command_id VARCHAR(64) NOT NULL, 
	created_at DATETIME NOT NULL, 
	PRIMARY KEY (id), 
	FOREIGN KEY(challenge_id, experiment_id) REFERENCES artifacts (challenge_id, artifact_id) ON DELETE CASCADE, 
	UNIQUE (command_id)
);
CREATE INDEX IF NOT EXISTS ix_observation_experiment ON observations (challenge_id, experiment_id);

CREATE TABLE IF NOT EXISTS mcp_connections (
	id VARCHAR(36) NOT NULL,
	owner_subject VARCHAR(300) NOT NULL,
	owner_name VARCHAR(240) NOT NULL,
	name VARCHAR(120) NOT NULL,
	server_url TEXT NOT NULL,
	auth_kind VARCHAR(16) NOT NULL,
	header_name VARCHAR(120),
	status VARCHAR(24) NOT NULL,
	last_error TEXT,
	secret_ciphertext TEXT,
	scopes TEXT,
	token_expires_at DATETIME,
	connected_at DATETIME,
	created_at DATETIME NOT NULL,
	updated_at DATETIME NOT NULL,
	PRIMARY KEY (id),
	CONSTRAINT uq_mcp_connection_owner_name UNIQUE (owner_subject, name)
);
CREATE INDEX IF NOT EXISTS ix_mcp_connection_owner ON mcp_connections (owner_subject, created_at);

CREATE TABLE IF NOT EXISTS project_mcp_connections (
	challenge_id VARCHAR(36) NOT NULL,
	connection_id VARCHAR(36) NOT NULL,
	server_key VARCHAR(64) NOT NULL,
	granted_as VARCHAR(16) NOT NULL,
	enabled_by VARCHAR(200) NOT NULL,
	created_at DATETIME NOT NULL,
	PRIMARY KEY (challenge_id, connection_id),
	FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE,
	FOREIGN KEY(connection_id) REFERENCES mcp_connections (id) ON DELETE CASCADE,
	CONSTRAINT uq_project_mcp_server_key UNIQUE (challenge_id, server_key)
);
CREATE INDEX IF NOT EXISTS ix_project_mcp_connection ON project_mcp_connections (connection_id);

CREATE TABLE IF NOT EXISTS mcp_authorization_flows (
	state_hash VARCHAR(64) NOT NULL,
	connection_id VARCHAR(36) NOT NULL,
	subject VARCHAR(300) NOT NULL,
	browser_hash VARCHAR(64) NOT NULL,
	redirect_uri TEXT NOT NULL,
	return_to TEXT NOT NULL,
	payload_ciphertext TEXT NOT NULL,
	created_at DATETIME NOT NULL,
	expires_at DATETIME NOT NULL,
	PRIMARY KEY (state_hash),
	FOREIGN KEY(connection_id) REFERENCES mcp_connections (id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS ix_mcp_flow_connection ON mcp_authorization_flows (connection_id);
CREATE INDEX IF NOT EXISTS ix_mcp_flow_expiry ON mcp_authorization_flows (expires_at);


CREATE TABLE IF NOT EXISTS project_metrics (
    challenge_id VARCHAR(36) NOT NULL REFERENCES challenges(id) ON DELETE CASCADE,
    metric_id VARCHAR(120) NOT NULL,
    payload JSON NOT NULL,
    version INTEGER NOT NULL,
    updated_by VARCHAR(200) NOT NULL,
    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL,
    PRIMARY KEY (challenge_id, metric_id)
);
CREATE INDEX IF NOT EXISTS ix_artifact_parent_hypothesis ON artifacts (challenge_id, parent_hypothesis_id, updated_at);
CREATE INDEX IF NOT EXISTS ix_artifact_parent_experiment ON artifacts (challenge_id, parent_experiment_id, updated_at);
