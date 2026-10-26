-- Alembic revision 5b8e1d3f7a20 (previous: 1f6a2c8d9e10)

CREATE TABLE project_engine_accounts (
    challenge_id VARCHAR(36) NOT NULL, 
    engine VARCHAR(32) NOT NULL, 
    method VARCHAR(32) NOT NULL, 
    label VARCHAR(320), 
    secret_ciphertext TEXT, 
    connected_by VARCHAR(200) NOT NULL, 
    connected_by_subject VARCHAR(300) NOT NULL, 
    created_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL, 
    PRIMARY KEY (challenge_id, engine), 
    FOREIGN KEY(challenge_id) REFERENCES challenges (id) ON DELETE CASCADE
);
