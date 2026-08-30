ALTER TABLE runs
    ADD COLUMN IF NOT EXISTS workspace_path TEXT,
    ADD COLUMN IF NOT EXISTS workspace_branch TEXT,
    ADD COLUMN IF NOT EXISTS workspace_base_ref TEXT,
    ADD COLUMN IF NOT EXISTS workspace_base_commit TEXT,
    ADD COLUMN IF NOT EXISTS workspace_disposition TEXT,
    ADD COLUMN IF NOT EXISTS workspace_detail TEXT;

ALTER TABLE runs
    DROP CONSTRAINT IF EXISTS runs_workspace_disposition_check;

ALTER TABLE runs
    ADD CONSTRAINT runs_workspace_disposition_check
    CHECK (
        workspace_disposition IS NULL
        OR workspace_disposition IN ('active', 'preserved', 'removed')
    );
