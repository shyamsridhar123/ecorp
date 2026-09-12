-- A provider correction is not checkpoint verification and never inherits its
-- zero-model-allocation exemption. Keep the existing checkpoint constraint.
ALTER TABLE factory_verification_recoveries
    ADD COLUMN source_correction_authority JSONB;

ALTER TABLE factory_verification_recoveries
    ADD CONSTRAINT factory_source_correction_authority_check
    CHECK (
        source_correction_authority IS NULL
        OR (
            mode = 'source_correction'
            AND jsonb_typeof(source_correction_authority) = 'object'
            AND source_correction_authority ->> 'schema_version' = '1'
        )
    );
