CREATE TABLE project_metrics (
    challenge_id VARCHAR(36) NOT NULL REFERENCES challenges(id) ON DELETE CASCADE,
    metric_id VARCHAR(120) NOT NULL,
    payload JSON NOT NULL,
    version INTEGER NOT NULL,
    updated_by VARCHAR(200) NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL,
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL,
    PRIMARY KEY (challenge_id, metric_id)
);
CREATE INDEX ix_artifact_parent_hypothesis ON artifacts (challenge_id, parent_hypothesis_id, updated_at);
CREATE INDEX ix_artifact_parent_experiment ON artifacts (challenge_id, parent_experiment_id, updated_at);
