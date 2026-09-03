ALTER TABLE missions
    DROP CONSTRAINT IF EXISTS missions_graph_limits_check;

ALTER TABLE missions
    ADD CONSTRAINT missions_graph_limits_check
    CHECK (
        max_nodes BETWEEN 1 AND 32
        AND max_depth BETWEEN 0 AND 8
        AND budget_tokens BETWEEN 1 AND 20000000
    );
