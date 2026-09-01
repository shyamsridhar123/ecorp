ALTER TABLE factory_operations
    DROP CONSTRAINT IF EXISTS factory_operations_operation_check;

ALTER TABLE factory_operations
    ADD CONSTRAINT factory_operations_operation_check
    CHECK (operation IN ('claim', 'renew', 'materialize', 'transition'));
