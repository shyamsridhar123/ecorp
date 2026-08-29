CREATE TABLE IF NOT EXISTS corps (
    id UUID PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS actors (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('human', 'agent', 'service')),
    role TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS actors_corp_idx ON actors(corp_id);

CREATE TABLE IF NOT EXISTS rooms (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    purpose TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (corp_id, name)
);

CREATE TABLE IF NOT EXISTS agents (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    actor_id UUID NOT NULL UNIQUE REFERENCES actors(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    role TEXT NOT NULL,
    adapter TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'idle',
    station TEXT,
    current_run_id UUID,
    accent TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS agents_corp_idx ON agents(corp_id);

CREATE TABLE IF NOT EXISTS missions (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    room_id UUID NOT NULL REFERENCES rooms(id) ON DELETE RESTRICT,
    requested_by UUID NOT NULL REFERENCES actors(id) ON DELETE RESTRICT,
    title TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS missions_corp_idx ON missions(corp_id, created_at DESC);

CREATE TABLE IF NOT EXISTS tasks (
    id UUID PRIMARY KEY,
    mission_id UUID NOT NULL REFERENCES missions(id) ON DELETE CASCADE,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    objective TEXT NOT NULL,
    status TEXT NOT NULL,
    assigned_agent_id UUID REFERENCES agents(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS tasks_mission_idx ON tasks(mission_id);

CREATE TABLE IF NOT EXISTS runs (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE RESTRICT,
    runner_id TEXT NOT NULL,
    status TEXT NOT NULL,
    summary TEXT,
    artifact_path TEXT,
    artifact_sha256 TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS runs_corp_idx ON runs(corp_id, created_at DESC);
CREATE INDEX IF NOT EXISTS runs_agent_active_idx ON runs(agent_id, status);

CREATE TABLE IF NOT EXISTS control_leases (
    agent_id UUID PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    actor_id UUID NOT NULL REFERENCES actors(id) ON DELETE CASCADE,
    token UUID NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS control_leases_corp_idx ON control_leases(corp_id);

CREATE TABLE IF NOT EXISTS queued_messages (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    actor_id UUID NOT NULL REFERENCES actors(id) ON DELETE CASCADE,
    text TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS queued_messages_agent_idx
    ON queued_messages(agent_id, created_at DESC);

CREATE TABLE IF NOT EXISTS events (
    seq BIGSERIAL PRIMARY KEY,
    id UUID NOT NULL UNIQUE,
    schema_version INTEGER NOT NULL DEFAULT 1,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    room_id UUID REFERENCES rooms(id) ON DELETE SET NULL,
    actor_id UUID REFERENCES actors(id) ON DELETE SET NULL,
    type TEXT NOT NULL,
    aggregate_type TEXT NOT NULL,
    aggregate_id UUID NOT NULL,
    aggregate_version BIGINT NOT NULL DEFAULT 1,
    correlation_id UUID,
    causation_id UUID,
    idempotency_key TEXT NOT NULL,
    visibility TEXT NOT NULL DEFAULT 'corp',
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (corp_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS events_corp_seq_idx ON events(corp_id, seq DESC);
CREATE INDEX IF NOT EXISTS events_aggregate_idx
    ON events(corp_id, aggregate_type, aggregate_id, aggregate_version DESC);

