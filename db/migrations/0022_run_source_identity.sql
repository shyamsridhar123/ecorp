ALTER TABLE runs
    ADD COLUMN IF NOT EXISTS source_repository TEXT,
    ADD COLUMN IF NOT EXISTS source_base_ref TEXT,
    ADD COLUMN IF NOT EXISTS source_base_commit TEXT;

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
            'upgrade_source_commit'
        )
    );

WITH task_commits AS (
    SELECT
        t.id AS task_id,
        min(lower(r.workspace_base_commit)) AS source_base_commit
    FROM tasks t
    JOIN runs r ON r.task_id = t.id
    WHERE t.contract->>'source_repository' IS NOT NULL
      AND t.contract->>'source_base_ref' IS NOT NULL
      AND t.contract->>'source_base_commit' IS NULL
      AND (
          r.workspace_base_commit ~ '^[0-9A-Fa-f]{40}$'
          OR r.workspace_base_commit ~ '^[0-9A-Fa-f]{64}$'
      )
    GROUP BY t.id
    HAVING count(DISTINCT lower(r.workspace_base_commit)) = 1
)
UPDATE tasks t
SET contract = jsonb_set(
        t.contract,
        '{source_base_commit}',
        to_jsonb(task_commits.source_base_commit),
        true
    ),
    updated_at = now()
FROM task_commits
WHERE t.id = task_commits.task_id;

WITH factory_commits AS (
    SELECT
        f.id AS work_item_id,
        min(lower(r.workspace_base_commit)) AS source_base_commit
    FROM factory_work_items f
    JOIN tasks t ON t.mission_id = f.mission_id
    JOIN runs r ON r.task_id = t.id
    WHERE f.policy->>'source_base_ref' IS NOT NULL
      AND f.policy->>'source_base_commit' IS NULL
      AND (
          r.workspace_base_commit ~ '^[0-9A-Fa-f]{40}$'
          OR r.workspace_base_commit ~ '^[0-9A-Fa-f]{64}$'
      )
    GROUP BY f.id
    HAVING count(DISTINCT lower(r.workspace_base_commit)) = 1
)
UPDATE factory_work_items f
SET policy = jsonb_set(
        jsonb_set(
            f.policy,
            '{source_base_commit}',
            to_jsonb(factory_commits.source_base_commit),
            true
        ),
        '{source_commit_upgrade_required}',
        'false'::jsonb,
        true
    ),
    updated_at = now()
FROM factory_commits
WHERE f.id = factory_commits.work_item_id;

UPDATE tasks t
SET contract = jsonb_set(
        t.contract,
        '{source_base_commit}',
        to_jsonb(f.policy->>'source_base_commit'),
        true
    ),
    updated_at = now()
FROM factory_work_items f
WHERE t.mission_id = f.mission_id
  AND t.contract->>'source_repository' =
      concat(f.source_repository_owner, '/', f.source_repository_name)
  AND t.contract->>'source_base_ref' = f.policy->>'source_base_ref'
  AND t.contract->>'source_base_commit' IS NULL
  AND (
      f.policy->>'source_base_commit' ~ '^[0-9A-Fa-f]{40}$'
      OR f.policy->>'source_base_commit' ~ '^[0-9A-Fa-f]{64}$'
  );

UPDATE factory_work_items
SET policy = jsonb_set(
        policy,
        '{source_commit_upgrade_required}',
        'true'::jsonb,
        true
    ),
    updated_at = now()
WHERE policy->>'source_base_ref' IS NOT NULL
  AND policy->>'source_base_commit' IS NULL;

UPDATE factory_work_items
SET policy = jsonb_set(
        policy,
        '{source_commit_upgrade_required}',
        'false'::jsonb,
        true
    ),
    updated_at = now()
WHERE policy->>'source_base_commit' IS NOT NULL
  AND policy->>'source_commit_upgrade_required' IS DISTINCT FROM 'false';

UPDATE runs r
SET source_repository = t.contract->>'source_repository',
    source_base_ref = t.contract->>'source_base_ref',
    source_base_commit = lower(t.contract->>'source_base_commit')
FROM tasks t
WHERE r.task_id = t.id
  AND r.source_repository IS NULL
  AND r.source_base_ref IS NULL
  AND r.source_base_commit IS NULL
  AND t.contract->>'source_repository' IS NOT NULL
  AND t.contract->>'source_base_ref' IS NOT NULL
  AND (
      t.contract->>'source_base_commit' ~ '^[0-9A-Fa-f]{40}$'
      OR t.contract->>'source_base_commit' ~ '^[0-9A-Fa-f]{64}$'
  );

ALTER TABLE runs
    DROP CONSTRAINT IF EXISTS runs_source_identity_check;

ALTER TABLE runs
    ADD CONSTRAINT runs_source_identity_check
    CHECK (
        (
            source_repository IS NULL
            AND source_base_ref IS NULL
            AND source_base_commit IS NULL
        )
        OR
        (
            source_repository IS NOT NULL
            AND source_base_ref IS NOT NULL
            AND source_base_commit IS NOT NULL
            AND (
                source_base_commit ~ '^[0-9A-Fa-f]{40}$'
                OR source_base_commit ~ '^[0-9A-Fa-f]{64}$'
            )
        )
    ) NOT VALID;

ALTER TABLE runs
    VALIDATE CONSTRAINT runs_source_identity_check;
