ALTER TABLE missions
    ADD COLUMN IF NOT EXISTS strategy TEXT NOT NULL DEFAULT 'single',
    ADD COLUMN IF NOT EXISTS max_nodes INTEGER NOT NULL DEFAULT 1,
    ADD COLUMN IF NOT EXISTS max_depth INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS budget_tokens BIGINT NOT NULL DEFAULT 100000;

ALTER TABLE tasks
    ADD COLUMN IF NOT EXISTS plan_key TEXT,
    ADD COLUMN IF NOT EXISTS contract JSONB,
    ADD COLUMN IF NOT EXISTS depth INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS max_attempts INTEGER NOT NULL DEFAULT 1,
    ADD COLUMN IF NOT EXISTS attempt_count INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS required_adapter TEXT;

UPDATE tasks
SET plan_key = id::text
WHERE plan_key IS NULL;

UPDATE tasks
SET contract = jsonb_build_object(
    'objective', objective,
    'expected_output', 'a source-backed artifact',
    'acceptance_tests', jsonb_build_array('artifact exists'),
    'allowed_tools', jsonb_build_array('filesystem', 'shell'),
    'prohibited_actions', jsonb_build_array('modify files outside the assigned worktree'),
    'references', '[]'::jsonb,
    'write_scope', jsonb_build_array('**'),
    'budget_tokens', 100000,
    'deadline_at', NULL,
    'escalation', 'ask a human operator'
)
WHERE contract IS NULL;

ALTER TABLE tasks
    ALTER COLUMN plan_key SET NOT NULL,
    ALTER COLUMN contract SET NOT NULL;

ALTER TABLE missions
    DROP CONSTRAINT IF EXISTS missions_graph_limits_check;

ALTER TABLE missions
    ADD CONSTRAINT missions_graph_limits_check
    CHECK (
        max_nodes BETWEEN 1 AND 32
        AND max_depth BETWEEN 0 AND 8
        AND budget_tokens BETWEEN 1 AND 2000000
    );

ALTER TABLE tasks
    DROP CONSTRAINT IF EXISTS tasks_execution_limits_check;

ALTER TABLE tasks
    ADD CONSTRAINT tasks_execution_limits_check
    CHECK (
        depth BETWEEN 0 AND 8
        AND max_attempts BETWEEN 1 AND 5
        AND attempt_count BETWEEN 0 AND max_attempts
    );

CREATE UNIQUE INDEX IF NOT EXISTS tasks_mission_plan_key_idx
    ON tasks(mission_id, plan_key);

CREATE TABLE IF NOT EXISTS task_dependencies (
    task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    depends_on_task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (task_id, depends_on_task_id),
    CHECK (task_id <> depends_on_task_id)
);

CREATE INDEX IF NOT EXISTS task_dependencies_parent_idx
    ON task_dependencies(depends_on_task_id, task_id);
