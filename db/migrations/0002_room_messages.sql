CREATE TABLE IF NOT EXISTS room_memberships (
    room_id UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    actor_id UUID NOT NULL REFERENCES actors(id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'member',
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (room_id, actor_id)
);

CREATE INDEX IF NOT EXISTS room_memberships_actor_idx
    ON room_memberships(actor_id, room_id);

CREATE TABLE IF NOT EXISTS room_messages (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    room_id UUID NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    actor_id UUID NOT NULL REFERENCES actors(id) ON DELETE RESTRICT,
    thread_root_id UUID REFERENCES room_messages(id) ON DELETE CASCADE,
    reply_to_id UUID REFERENCES room_messages(id) ON DELETE SET NULL,
    body TEXT NOT NULL CHECK (char_length(body) BETWEEN 1 AND 4000),
    mentions UUID[] NOT NULL DEFAULT '{}'::uuid[],
    link_kind TEXT,
    link_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (
        (link_kind IS NULL AND link_id IS NULL)
        OR (link_kind IS NOT NULL AND link_id IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS room_messages_room_created_idx
    ON room_messages(room_id, created_at);

CREATE INDEX IF NOT EXISTS room_messages_thread_idx
    ON room_messages(thread_root_id, created_at)
    WHERE thread_root_id IS NOT NULL;

