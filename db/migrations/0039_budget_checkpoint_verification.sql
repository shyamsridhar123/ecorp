-- An explicit operation within the existing recovery aggregate. It grants
-- provider-free verification, never another provider run or a budget reset.
ALTER TABLE factory_verification_recoveries
    DROP CONSTRAINT factory_verification_recoveries_mode_check;

ALTER TABLE factory_verification_recoveries
    ADD CONSTRAINT factory_verification_recoveries_mode_check
    CHECK (mode IN ('source_correction', 'verifier_only', 'checkpoint_verification'));

ALTER TABLE factory_verification_recoveries
    ADD COLUMN checkpoint_authority JSONB;

ALTER TABLE factory_verification_recoveries
    ADD CONSTRAINT factory_checkpoint_authority_check
    CHECK (
        (mode = 'checkpoint_verification') = (checkpoint_authority IS NOT NULL)
    );
