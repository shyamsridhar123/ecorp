ALTER TABLE queued_messages
    ADD COLUMN IF NOT EXISTS run_id UUID REFERENCES runs(id) ON DELETE SET NULL;

ALTER TABLE queued_messages
    ADD COLUMN IF NOT EXISTS delivered_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS queued_messages_delivery_idx
    ON queued_messages(agent_id, status, created_at);
