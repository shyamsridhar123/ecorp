CREATE TABLE IF NOT EXISTS runner_nodes (
    id TEXT PRIMARY KEY,
    hostname TEXT NOT NULL,
    os TEXT NOT NULL,
    capabilities JSONB NOT NULL DEFAULT '[]'::jsonb,
    connection_epoch UUID NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('connected', 'grace', 'offline')),
    connected_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    disconnected_at TIMESTAMPTZ,
    grace_expires_at TIMESTAMPTZ
);

ALTER TABLE runs
    ADD COLUMN IF NOT EXISTS assignment_token UUID;

UPDATE runs
SET assignment_token = gen_random_uuid()
WHERE assignment_token IS NULL;

ALTER TABLE runs
    ALTER COLUMN assignment_token SET NOT NULL;

CREATE INDEX IF NOT EXISTS runs_runner_active_idx
    ON runs(runner_id, status);

CREATE INDEX IF NOT EXISTS runner_nodes_status_idx
    ON runner_nodes(status, grace_expires_at);

