CREATE TABLE IF NOT EXISTS human_identities (
    issuer TEXT NOT NULL,
    subject TEXT NOT NULL,
    actor_id UUID NOT NULL UNIQUE REFERENCES actors(id) ON DELETE CASCADE,
    email TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_authenticated_at TIMESTAMPTZ,
    PRIMARY KEY (issuer, subject)
);

CREATE INDEX IF NOT EXISTS human_identities_actor_idx
    ON human_identities(actor_id);

UPDATE actors
SET role = 'member'
WHERE kind = 'human' AND role = 'reviewer';

DELETE FROM runner_nodes;

ALTER TABLE runner_nodes
    ADD COLUMN IF NOT EXISTS corp_id UUID REFERENCES corps(id) ON DELETE CASCADE;

ALTER TABLE runner_nodes
    ALTER COLUMN corp_id SET NOT NULL;

CREATE INDEX IF NOT EXISTS runner_nodes_corp_status_idx
    ON runner_nodes(corp_id, status, grace_expires_at);

CREATE TABLE IF NOT EXISTS runner_enrollment_tokens (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    runner_id TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    created_by UUID NOT NULL REFERENCES actors(id) ON DELETE RESTRICT,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS runner_enrollment_tokens_lookup_idx
    ON runner_enrollment_tokens(corp_id, runner_id, expires_at)
    WHERE used_at IS NULL;

CREATE TABLE IF NOT EXISTS runner_credentials (
    id UUID NOT NULL UNIQUE,
    runner_id TEXT PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    enrolled_by UUID NOT NULL REFERENCES actors(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    rotated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS runner_credentials_corp_idx
    ON runner_credentials(corp_id, runner_id);
