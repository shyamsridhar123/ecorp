ALTER TABLE runs
    ADD COLUMN IF NOT EXISTS workspace_run_id UUID;

UPDATE runs
SET workspace_run_id = id
WHERE workspace_run_id IS NULL;

ALTER TABLE runs
    ALTER COLUMN workspace_run_id SET NOT NULL;

ALTER TABLE runs
    DROP CONSTRAINT IF EXISTS runs_workspace_run_id_fkey;

ALTER TABLE runs
    ADD CONSTRAINT runs_workspace_run_id_fkey
    FOREIGN KEY (workspace_run_id) REFERENCES runs(id) ON DELETE CASCADE;
