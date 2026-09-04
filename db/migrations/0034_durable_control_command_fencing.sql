ALTER TABLE control_leases
    ADD COLUMN IF NOT EXISTS lease_version BIGINT NOT NULL DEFAULT 1;

ALTER TABLE runner_commands
    ADD COLUMN IF NOT EXISTS failure_detail TEXT,
    ADD COLUMN IF NOT EXISTS failed_at TIMESTAMPTZ;

ALTER TABLE runner_commands
    DROP CONSTRAINT IF EXISTS runner_commands_status_check;

ALTER TABLE runner_commands
    ADD CONSTRAINT runner_commands_status_check
    CHECK (status IN ('pending', 'dispatched', 'failed'));
