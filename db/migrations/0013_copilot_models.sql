ALTER TABLE runs
    ADD COLUMN IF NOT EXISTS model TEXT,
    ADD COLUMN IF NOT EXISTS reasoning_effort TEXT;

ALTER TABLE runs
    DROP CONSTRAINT IF EXISTS runs_reasoning_effort_check;

ALTER TABLE runs
    ADD CONSTRAINT runs_reasoning_effort_check
    CHECK (
        reasoning_effort IS NULL
        OR reasoning_effort IN ('low', 'medium', 'high', 'xhigh', 'max')
    );
