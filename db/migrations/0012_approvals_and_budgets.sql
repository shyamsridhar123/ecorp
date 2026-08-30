CREATE TABLE IF NOT EXISTS action_approvals (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    room_id UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    mission_id UUID NOT NULL REFERENCES missions(id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    run_id UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE RESTRICT,
    action_key TEXT NOT NULL,
    action TEXT NOT NULL,
    risk TEXT NOT NULL CHECK (risk IN ('low', 'medium', 'high', 'critical')),
    rationale TEXT NOT NULL,
    required_roles TEXT[] NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'approved', 'rejected', 'expired')),
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_by UUID REFERENCES actors(id) ON DELETE SET NULL,
    decision_note TEXT,
    decision_key UUID UNIQUE,
    decided_at TIMESTAMPTZ,
    UNIQUE (run_id, action_key)
);

CREATE INDEX IF NOT EXISTS action_approvals_corp_status_idx
    ON action_approvals(corp_id, status, created_at);

CREATE TABLE IF NOT EXISTS runner_commands (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    runner_id TEXT NOT NULL,
    run_id UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    command_kind TEXT NOT NULL,
    payload JSONB NOT NULL,
    idempotency_key TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'dispatched')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    dispatched_at TIMESTAMPTZ,
    UNIQUE (corp_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS runner_commands_pending_idx
    ON runner_commands(runner_id, status, created_at);

ALTER TABLE missions
    ADD COLUMN IF NOT EXISTS budget_cost_microusd BIGINT NOT NULL DEFAULT 5000000;

ALTER TABLE runs
    ADD COLUMN IF NOT EXISTS budget_tokens_limit BIGINT NOT NULL DEFAULT 100000,
    ADD COLUMN IF NOT EXISTS budget_cost_microusd_limit BIGINT NOT NULL DEFAULT 1000000,
    ADD COLUMN IF NOT EXISTS breaker_stage TEXT NOT NULL DEFAULT 'healthy',
    ADD COLUMN IF NOT EXISTS no_progress_events INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS repeated_tool_count INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS last_tool_signature TEXT,
    ADD COLUMN IF NOT EXISTS last_progress_at TIMESTAMPTZ NOT NULL DEFAULT now();

ALTER TABLE runs
    DROP CONSTRAINT IF EXISTS runs_breaker_stage_check;

ALTER TABLE runs
    ADD CONSTRAINT runs_breaker_stage_check
    CHECK (breaker_stage IN ('healthy', 'steer', 'constrain', 'suspend', 'stop'));

CREATE TABLE IF NOT EXISTS corp_budget_policies (
    corp_id UUID PRIMARY KEY REFERENCES corps(id) ON DELETE CASCADE,
    actor_tokens_per_24h BIGINT NOT NULL DEFAULT 500000,
    actor_cost_microusd_per_24h BIGINT NOT NULL DEFAULT 10000000,
    corp_tokens_per_24h BIGINT NOT NULL DEFAULT 5000000,
    corp_cost_microusd_per_24h BIGINT NOT NULL DEFAULT 100000000,
    no_progress_event_limit INTEGER NOT NULL DEFAULT 8,
    repeated_tool_limit INTEGER NOT NULL DEFAULT 5,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS circuit_breaker_incidents (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    mission_id UUID NOT NULL REFERENCES missions(id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    run_id UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    stage TEXT NOT NULL CHECK (stage IN ('steer', 'constrain', 'suspend', 'stop')),
    reason TEXT NOT NULL,
    input JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (run_id, stage)
);
