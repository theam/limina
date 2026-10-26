-- Alembic revision c0d3c1a2b7e9 (previous: 9a62d4f771c1)

ALTER TABLE challenges ADD COLUMN runtime_engine VARCHAR(32) DEFAULT 'codex' NOT NULL;

ALTER TABLE coordinator_states RENAME thread_id TO continuation_id;
