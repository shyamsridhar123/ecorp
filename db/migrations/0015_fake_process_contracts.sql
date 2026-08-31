UPDATE tasks
SET contract = contract - 'model' - 'reasoning_effort',
    updated_at = now()
WHERE required_adapter = 'fake-process'
  AND (
      contract->>'model' IS NOT NULL
      OR contract->>'reasoning_effort' IS NOT NULL
  );

ALTER TABLE tasks
    DROP CONSTRAINT IF EXISTS tasks_fake_process_contract_check;

ALTER TABLE tasks
    ADD CONSTRAINT tasks_fake_process_contract_check
    CHECK (
        required_adapter IS DISTINCT FROM 'fake-process'
        OR (
            contract->>'model' IS NULL
            AND contract->>'reasoning_effort' IS NULL
        )
    );
