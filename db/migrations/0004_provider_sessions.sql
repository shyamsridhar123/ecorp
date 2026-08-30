ALTER TABLE runs
    ADD COLUMN IF NOT EXISTS provider_session_id TEXT,
    ADD COLUMN IF NOT EXISTS resumed_from_run_id UUID REFERENCES runs(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS input_tokens BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS output_tokens BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS cost_microusd BIGINT NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS runs_provider_session_idx
    ON runs(provider_session_id)
    WHERE provider_session_id IS NOT NULL;
