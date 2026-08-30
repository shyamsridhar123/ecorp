ALTER TABLE tasks
    ADD COLUMN IF NOT EXISTS verification_policy JSONB,
    ADD COLUMN IF NOT EXISTS verification_status TEXT NOT NULL DEFAULT 'pending';

UPDATE tasks
SET verification_policy = jsonb_build_object(
    'checks', jsonb_build_array(
        jsonb_build_object('type', 'artifact', 'min_bytes', 1)
    ),
    'manual_gate', NULL
)
WHERE verification_policy IS NULL;

ALTER TABLE tasks
    ALTER COLUMN verification_policy SET NOT NULL;

ALTER TABLE runs
    ADD COLUMN IF NOT EXISTS verification_status TEXT NOT NULL DEFAULT 'pending',
    ADD COLUMN IF NOT EXISTS verification_summary TEXT;

ALTER TABLE tasks
    DROP CONSTRAINT IF EXISTS tasks_verification_status_check;

ALTER TABLE tasks
    ADD CONSTRAINT tasks_verification_status_check
    CHECK (
        verification_status IN (
            'pending', 'running', 'passed', 'failed', 'waiting_for_approval'
        )
    );

ALTER TABLE runs
    DROP CONSTRAINT IF EXISTS runs_verification_status_check;

ALTER TABLE runs
    ADD CONSTRAINT runs_verification_status_check
    CHECK (
        verification_status IN (
            'pending', 'running', 'passed', 'failed', 'waiting_for_approval'
        )
    );

CREATE TABLE IF NOT EXISTS verification_evidence (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    run_id UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    check_index INTEGER NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('passed', 'failed')),
    summary TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (run_id, check_index)
);

CREATE INDEX IF NOT EXISTS verification_evidence_task_idx
    ON verification_evidence(task_id, created_at);

CREATE TABLE IF NOT EXISTS verification_requests (
    run_id UUID PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    gate_type TEXT NOT NULL CHECK (gate_type IN ('human_approval', 'independent_review')),
    gate JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'approved', 'rejected')),
    requested_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_by UUID REFERENCES actors(id) ON DELETE SET NULL,
    decision_note TEXT,
    decided_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS verification_requests_corp_status_idx
    ON verification_requests(corp_id, status, requested_at);
