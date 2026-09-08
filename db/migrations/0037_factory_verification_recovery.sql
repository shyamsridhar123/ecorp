ALTER TABLE verification_requests
    ADD COLUMN IF NOT EXISTS decision_key UUID;

CREATE UNIQUE INDEX IF NOT EXISTS verification_requests_decision_key_idx
    ON verification_requests(corp_id, decision_key)
    WHERE decision_key IS NOT NULL;

ALTER TABLE factory_operations
    DROP CONSTRAINT IF EXISTS factory_operations_operation_check;

ALTER TABLE factory_operations
    ADD CONSTRAINT factory_operations_operation_check
    CHECK (
        operation IN (
            'claim',
            'renew',
            'materialize',
            'transition',
            'upgrade_source_commit',
            'materialize_rejected',
            'checkpoint_workspace'
        )
    );

ALTER TABLE runs
    ADD COLUMN IF NOT EXISTS execution_mode TEXT NOT NULL DEFAULT 'provider',
    ADD COLUMN IF NOT EXISTS workspace_fingerprint TEXT;

ALTER TABLE runs
    DROP CONSTRAINT IF EXISTS runs_execution_mode_check;

ALTER TABLE runs
    ADD CONSTRAINT runs_execution_mode_check
    CHECK (execution_mode IN ('provider', 'verification_only'));

ALTER TABLE runs
    DROP CONSTRAINT IF EXISTS runs_workspace_fingerprint_check;

ALTER TABLE runs
    ADD CONSTRAINT runs_workspace_fingerprint_check
    CHECK (
        workspace_fingerprint IS NULL
        OR workspace_fingerprint ~ '^[0-9a-f]{64}$'
    );

CREATE TABLE IF NOT EXISTS factory_verification_recoveries (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    factory_work_item_id UUID NOT NULL REFERENCES factory_work_items(id) ON DELETE CASCADE,
    mission_id UUID NOT NULL REFERENCES missions(id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    source_run_id UUID NOT NULL REFERENCES runs(id) ON DELETE RESTRICT,
    replacement_run_id UUID UNIQUE REFERENCES runs(id) ON DELETE RESTRICT,
    mode TEXT NOT NULL CHECK (mode IN ('source_correction', 'verifier_only')),
    status TEXT NOT NULL CHECK (status IN ('authorized', 'running', 'completed', 'failed')),
    authorized_by UUID NOT NULL REFERENCES actors(id) ON DELETE RESTRICT,
    reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 4000),
    idempotency_key UUID NOT NULL,
    observed_source_revision TEXT NOT NULL CHECK (length(observed_source_revision) BETWEEN 1 AND 200),
    reviewed_source_snapshot JSONB NOT NULL,
    contract_revision_id UUID REFERENCES mission_contract_revisions(id) ON DELETE RESTRICT,
    previous_verification_policy JSONB NOT NULL,
    replacement_verification_policy JSONB NOT NULL,
    request JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (corp_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS factory_verification_recoveries_item_idx
    ON factory_verification_recoveries(corp_id, factory_work_item_id, created_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS factory_verification_recoveries_one_active_idx
    ON factory_verification_recoveries(factory_work_item_id)
    WHERE status IN ('authorized', 'running');
