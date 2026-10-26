-- Alembic revision 1f6a2c8d9e10 (previous: 7e4a19b8d2c6)

ALTER TABLE runtime_runs ADD COLUMN reasoning_output_tokens BIGINT;

ALTER TABLE runtime_runs ADD COLUMN total_tokens BIGINT;

ALTER TABLE runtime_runs ADD COLUMN usage_source VARCHAR(40);

ALTER TABLE runtime_runs ADD COLUMN cost_source VARCHAR(40);
