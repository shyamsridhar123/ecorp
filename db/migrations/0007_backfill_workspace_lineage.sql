WITH RECURSIVE lineage AS (
    SELECT
        id AS descendant_id,
        id AS current_id,
        resumed_from_run_id,
        0 AS depth
    FROM runs

    UNION ALL

    SELECT
        lineage.descendant_id,
        parent.id AS current_id,
        parent.resumed_from_run_id,
        lineage.depth + 1
    FROM lineage
    JOIN runs parent ON parent.id = lineage.resumed_from_run_id
    WHERE lineage.resumed_from_run_id IS NOT NULL
      AND lineage.depth < 100
),
roots AS (
    SELECT DISTINCT ON (descendant_id)
        descendant_id,
        current_id AS root_id
    FROM lineage
    ORDER BY descendant_id, depth DESC
)
UPDATE runs
SET workspace_run_id = roots.root_id
FROM roots
WHERE runs.id = roots.descendant_id
  AND runs.workspace_run_id IS DISTINCT FROM roots.root_id;
