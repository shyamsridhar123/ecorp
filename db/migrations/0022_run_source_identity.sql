ALTER TABLE runs
    ADD COLUMN IF NOT EXISTS source_repository TEXT,
    ADD COLUMN IF NOT EXISTS source_base_ref TEXT,
    ADD COLUMN IF NOT EXISTS source_base_commit TEXT;

ALTER TABLE runs
    DROP CONSTRAINT IF EXISTS runs_source_identity_check;

ALTER TABLE runs
    ADD CONSTRAINT runs_source_identity_check
    CHECK (
        (
            source_repository IS NULL
            AND source_base_ref IS NULL
            AND source_base_commit IS NULL
        )
        OR
        (
            source_repository IS NOT NULL
            AND source_base_ref IS NOT NULL
            AND source_base_commit IS NOT NULL
            AND (
                source_base_commit ~ '^[0-9A-Fa-f]{40}$'
                OR source_base_commit ~ '^[0-9A-Fa-f]{64}$'
            )
        )
    ) NOT VALID;

ALTER TABLE runs
    VALIDATE CONSTRAINT runs_source_identity_check;
