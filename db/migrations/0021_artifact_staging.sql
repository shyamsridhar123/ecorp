ALTER TABLE artifacts
    ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'ready',
    ADD COLUMN IF NOT EXISTS staging_key TEXT,
    ADD COLUMN IF NOT EXISTS finalized_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS rejection_reason TEXT;

UPDATE artifacts
SET finalized_at = COALESCE(finalized_at, created_at)
WHERE status = 'ready';

ALTER TABLE artifacts
    DROP CONSTRAINT IF EXISTS artifacts_status_check;

ALTER TABLE artifacts
    ADD CONSTRAINT artifacts_status_check
    CHECK (status IN ('staged', 'ready', 'rejected')) NOT VALID;

ALTER TABLE artifacts
    VALIDATE CONSTRAINT artifacts_status_check;

ALTER TABLE artifacts
    DROP CONSTRAINT IF EXISTS artifacts_staging_state_check;

ALTER TABLE artifacts
    ADD CONSTRAINT artifacts_staging_state_check
    CHECK (
        (status = 'staged' AND staging_key IS NOT NULL AND finalized_at IS NULL)
        OR (status = 'ready' AND finalized_at IS NOT NULL)
        OR status = 'rejected'
    ) NOT VALID;

ALTER TABLE artifacts
    VALIDATE CONSTRAINT artifacts_staging_state_check;
