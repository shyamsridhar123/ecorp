ALTER TABLE missions
    ADD COLUMN IF NOT EXISTS original_budget_tokens BIGINT,
    ADD COLUMN IF NOT EXISTS original_budget_cost_microusd BIGINT;

UPDATE missions
SET original_budget_tokens = budget_tokens,
    original_budget_cost_microusd = budget_cost_microusd
WHERE original_budget_tokens IS NULL
   OR original_budget_cost_microusd IS NULL;

ALTER TABLE missions
    ALTER COLUMN original_budget_tokens SET NOT NULL,
    ALTER COLUMN original_budget_cost_microusd SET NOT NULL;

ALTER TABLE missions
    DROP CONSTRAINT IF EXISTS missions_original_budget_check;

ALTER TABLE missions
    ADD CONSTRAINT missions_original_budget_check
    CHECK (
        original_budget_tokens > 0
        AND original_budget_cost_microusd > 0
        AND budget_tokens >= original_budget_tokens
        AND budget_cost_microusd >= original_budget_cost_microusd
    ) NOT VALID;

ALTER TABLE missions
    VALIDATE CONSTRAINT missions_original_budget_check;

CREATE TABLE IF NOT EXISTS mission_budget_revisions (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    mission_id UUID NOT NULL REFERENCES missions(id) ON DELETE CASCADE,
    proposed_by UUID NOT NULL REFERENCES actors(id) ON DELETE RESTRICT,
    status TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'rejected')),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    current_budget_tokens BIGINT NOT NULL CHECK (current_budget_tokens > 0),
    current_budget_cost_microusd BIGINT NOT NULL CHECK (current_budget_cost_microusd > 0),
    proposed_budget_tokens BIGINT NOT NULL CHECK (proposed_budget_tokens > 0),
    proposed_budget_cost_microusd BIGINT NOT NULL CHECK (proposed_budget_cost_microusd > 0),
    consumed_tokens_at_proposal BIGINT NOT NULL CHECK (consumed_tokens_at_proposal >= 0),
    consumed_cost_microusd_at_proposal BIGINT NOT NULL
        CHECK (consumed_cost_microusd_at_proposal >= 0),
    rationale TEXT NOT NULL,
    replacement_task_id UUID REFERENCES tasks(id) ON DELETE RESTRICT,
    previous_contract JSONB,
    replacement_contract JSONB,
    previous_verification_policy JSONB,
    replacement_verification_policy JSONB,
    idempotency_key UUID NOT NULL,
    proposal_request JSONB NOT NULL,
    decided_by UUID REFERENCES actors(id) ON DELETE RESTRICT,
    decision_note TEXT,
    decision_key UUID,
    decision_request JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (corp_id, idempotency_key),
    CHECK (
        proposed_budget_tokens >= current_budget_tokens
        AND proposed_budget_cost_microusd >= current_budget_cost_microusd
        AND (
            proposed_budget_tokens > current_budget_tokens
            OR proposed_budget_cost_microusd > current_budget_cost_microusd
        )
    ),
    CHECK (
        (
            replacement_task_id IS NULL
            AND previous_contract IS NULL
            AND replacement_contract IS NULL
            AND previous_verification_policy IS NULL
            AND replacement_verification_policy IS NULL
        )
        OR
        (
            replacement_task_id IS NOT NULL
            AND previous_contract IS NOT NULL
            AND replacement_contract IS NOT NULL
            AND previous_verification_policy IS NOT NULL
            AND replacement_verification_policy IS NOT NULL
        )
    ),
    CHECK (
        (
            status = 'pending'
            AND decided_by IS NULL
            AND decision_note IS NULL
            AND decision_key IS NULL
            AND decision_request IS NULL
            AND decided_at IS NULL
        )
        OR
        (
            status IN ('approved', 'rejected')
            AND decided_by IS NOT NULL
            AND decision_note IS NOT NULL
            AND decision_key IS NOT NULL
            AND decision_request IS NOT NULL
            AND decided_at IS NOT NULL
        )
    )
);

CREATE INDEX IF NOT EXISTS mission_budget_revisions_mission_idx
    ON mission_budget_revisions(corp_id, mission_id, created_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS mission_budget_revisions_one_pending_idx
    ON mission_budget_revisions(corp_id, mission_id)
    WHERE status = 'pending';

CREATE UNIQUE INDEX IF NOT EXISTS mission_budget_revisions_decision_key_idx
    ON mission_budget_revisions(corp_id, decision_key)
    WHERE decision_key IS NOT NULL;
