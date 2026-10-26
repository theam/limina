-- Alembic revision 8d3f2b6c1a47 (previous: 5b8e1d3f7a20)

CREATE TABLE project_invitations (
    challenge_id VARCHAR(36) NOT NULL,
    email VARCHAR(320) NOT NULL,
    role VARCHAR(16) NOT NULL,
    organization_id VARCHAR(300),
    created_by VARCHAR(300) NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL,
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL,
    PRIMARY KEY (challenge_id, email),
    FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);

CREATE INDEX ix_project_invitation_email ON project_invitations (email);

ALTER TABLE live_tickets ADD COLUMN project_admin BOOLEAN DEFAULT false NOT NULL;
