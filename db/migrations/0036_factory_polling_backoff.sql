ALTER TABLE factory_controllers
    ADD COLUMN polling_state JSONB NOT NULL DEFAULT '{}'::jsonb
    CHECK (jsonb_typeof(polling_state) = 'object');
