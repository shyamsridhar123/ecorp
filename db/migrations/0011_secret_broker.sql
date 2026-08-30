CREATE TABLE IF NOT EXISTS secrets (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    ciphertext BYTEA NOT NULL,
    nonce BYTEA NOT NULL,
    allowed_actor_ids UUID[] NOT NULL,
    allowed_tools TEXT[] NOT NULL,
    resource_prefix TEXT NOT NULL,
    max_ttl_seconds INTEGER NOT NULL CHECK (max_ttl_seconds BETWEEN 30 AND 3600),
    created_by UUID NOT NULL REFERENCES actors(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at TIMESTAMPTZ,
    UNIQUE (corp_id, name)
);

CREATE TABLE IF NOT EXISTS secret_access_grants (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    secret_id UUID NOT NULL REFERENCES secrets(id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    run_id UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    actor_id UUID NOT NULL REFERENCES actors(id) ON DELETE RESTRICT,
    runner_id TEXT NOT NULL,
    tool TEXT NOT NULL,
    resource TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS secret_access_grants_run_idx
    ON secret_access_grants(run_id, created_at);
