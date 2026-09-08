-- A retained workspace that failed its integrity boundary is not an unsealed
-- legacy workspace. Its quarantine must survive cleanup replay and restart.
ALTER TABLE runs DROP CONSTRAINT runs_workspace_disposition_check;
ALTER TABLE runs ADD CONSTRAINT runs_workspace_disposition_check
    CHECK (
        workspace_disposition IS NULL
        OR workspace_disposition IN ('active', 'preserved', 'removed', 'quarantined')
    );
