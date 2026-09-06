ALTER TABLE room_messages
    ADD COLUMN IF NOT EXISTS idempotency_key UUID;

CREATE UNIQUE INDEX IF NOT EXISTS room_messages_corp_idempotency_idx
    ON room_messages(corp_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

ALTER TABLE queued_messages
    ADD COLUMN IF NOT EXISTS idempotency_key UUID,
    ADD COLUMN IF NOT EXISTS command_id UUID REFERENCES runner_commands(id) ON DELETE SET NULL;

CREATE UNIQUE INDEX IF NOT EXISTS queued_messages_corp_idempotency_idx
    ON queued_messages(corp_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS queued_messages_command_idx
    ON queued_messages(command_id)
    WHERE command_id IS NOT NULL;
