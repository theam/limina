-- Alembic revision 8c4f2a6e1d93 (previous: 8d3f2b6c1a47)

CREATE TABLE mcp_connections (
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
    token_expires_at TIMESTAMP WITH TIME ZONE,
    connected_at TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL,
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL,
    PRIMARY KEY (id),
    CONSTRAINT uq_mcp_connection_owner_name UNIQUE (owner_subject, name)
);

CREATE INDEX ix_mcp_connection_owner ON mcp_connections (owner_subject, created_at);

CREATE TABLE project_mcp_connections (
    challenge_id VARCHAR(36) NOT NULL,
    connection_id VARCHAR(36) NOT NULL,
    server_key VARCHAR(64) NOT NULL,
    granted_as VARCHAR(16) NOT NULL,
    enabled_by VARCHAR(200) NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL,
    PRIMARY KEY (challenge_id, connection_id),
    FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE,
    FOREIGN KEY(connection_id) REFERENCES mcp_connections (id) ON DELETE CASCADE,
    CONSTRAINT uq_project_mcp_server_key UNIQUE (challenge_id, server_key)
);

CREATE INDEX ix_project_mcp_connection ON project_mcp_connections (connection_id);

CREATE TABLE mcp_authorization_flows (
    state_hash VARCHAR(64) NOT NULL,
    connection_id VARCHAR(36) NOT NULL,
    subject VARCHAR(300) NOT NULL,
    browser_hash VARCHAR(64) NOT NULL,
    redirect_uri TEXT NOT NULL,
    return_to TEXT NOT NULL,
    payload_ciphertext TEXT NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL,
    expires_at TIMESTAMP WITH TIME ZONE NOT NULL,
    PRIMARY KEY (state_hash),
    FOREIGN KEY(connection_id) REFERENCES mcp_connections (id) ON DELETE CASCADE
);

CREATE INDEX ix_mcp_flow_connection ON mcp_authorization_flows (connection_id);

CREATE INDEX ix_mcp_flow_expiry ON mcp_authorization_flows (expires_at);
