CREATE TABLE IF NOT EXISTS publication_publisher_credentials (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    publisher_id TEXT NOT NULL CHECK (length(publisher_id) BETWEEN 1 AND 160),
    credential_hash TEXT NOT NULL UNIQUE
        CHECK (credential_hash ~ '^[0-9a-f]{64}$'),
    created_by UUID NOT NULL REFERENCES actors(id) ON DELETE RESTRICT,
    expires_at TIMESTAMPTZ NOT NULL,
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (expires_at > created_at)
);

CREATE INDEX IF NOT EXISTS publication_publisher_credentials_lookup_idx
    ON publication_publisher_credentials(corp_id, credential_hash, expires_at)
    WHERE revoked_at IS NULL;

CREATE INDEX IF NOT EXISTS publication_publisher_credentials_identity_idx
    ON publication_publisher_credentials(corp_id, publisher_id, expires_at DESC);
