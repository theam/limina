-- Alembic revision 9a62d4f771c1 (previous: 474c29565487)

ALTER TABLE project_resources ADD COLUMN resource_type VARCHAR(16);

ALTER TABLE project_resources ADD COLUMN value TEXT;

ALTER TABLE project_resources ADD COLUMN secret_ciphertext TEXT;

ALTER TABLE project_resources ADD COLUMN updated_at TIMESTAMP WITH TIME ZONE;

UPDATE project_resources
            SET name = upper(replace(name, '-', '_')),
                resource_type = 'VARIABLE',
                value = uri,
                updated_at = created_at;

ALTER TABLE project_resources ALTER COLUMN resource_type SET NOT NULL;

ALTER TABLE project_resources ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE project_resources DROP COLUMN credential_env;

ALTER TABLE project_resources DROP COLUMN kind;

ALTER TABLE project_resources DROP COLUMN uri;
