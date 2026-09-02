ALTER TABLE missions
    ADD COLUMN IF NOT EXISTS description TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS specification_version BIGINT NOT NULL DEFAULT 1;

ALTER TABLE missions
    DROP CONSTRAINT IF EXISTS missions_specification_check;

ALTER TABLE missions
    ADD CONSTRAINT missions_specification_check
    CHECK (
        length(description) <= 100000
        AND specification_version > 0
    );

ALTER TABLE tasks
    ADD COLUMN IF NOT EXISTS contract_version BIGINT NOT NULL DEFAULT 1;

ALTER TABLE tasks
    DROP CONSTRAINT IF EXISTS tasks_contract_version_check;

ALTER TABLE tasks
    ADD CONSTRAINT tasks_contract_version_check
    CHECK (contract_version > 0);

CREATE TABLE IF NOT EXISTS mission_contract_revisions (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    mission_id UUID NOT NULL REFERENCES missions(id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    version BIGINT NOT NULL CHECK (version > 1),
    revised_by UUID NOT NULL REFERENCES actors(id) ON DELETE RESTRICT,
    next_action TEXT NOT NULL CHECK (next_action IN ('redispatch', 'resume')),
    source_run_id UUID REFERENCES runs(id) ON DELETE RESTRICT,
    reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 4000),
    previous_description TEXT NOT NULL CHECK (length(previous_description) <= 100000),
    replacement_description TEXT NOT NULL CHECK (length(replacement_description) <= 100000),
    previous_contract JSONB NOT NULL,
    replacement_contract JSONB NOT NULL,
    previous_verification_policy JSONB NOT NULL,
    replacement_verification_policy JSONB NOT NULL,
    idempotency_key UUID NOT NULL,
    request JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (corp_id, idempotency_key),
    UNIQUE (task_id, version),
    CHECK (
        (next_action = 'redispatch' AND source_run_id IS NULL)
        OR
        (next_action = 'resume' AND source_run_id IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS mission_contract_revisions_mission_idx
    ON mission_contract_revisions(corp_id, mission_id, created_at DESC);
