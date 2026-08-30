CREATE TABLE IF NOT EXISTS artifacts (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    run_id UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    producer_agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE RESTRICT,
    producer_runner_id TEXT NOT NULL REFERENCES runner_nodes(id) ON DELETE RESTRICT,
    verifier TEXT NOT NULL,
    object_key TEXT NOT NULL,
    uri TEXT NOT NULL,
    sha256 TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    media_type TEXT NOT NULL CHECK (length(media_type) BETWEEN 3 AND 255),
    bytes BIGINT NOT NULL CHECK (bytes >= 0),
    retention_until TIMESTAMPTZ NOT NULL,
    provenance_signature TEXT NOT NULL CHECK (provenance_signature ~ '^[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (retention_until > created_at),
    UNIQUE (run_id, sha256)
);

CREATE INDEX IF NOT EXISTS artifacts_run_idx ON artifacts(run_id, created_at);
CREATE INDEX IF NOT EXISTS artifacts_corp_digest_idx ON artifacts(corp_id, sha256);

ALTER TABLE runs
    ADD COLUMN IF NOT EXISTS artifact_id UUID REFERENCES artifacts(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS artifact_uri TEXT,
    ADD COLUMN IF NOT EXISTS artifact_media_type TEXT,
    ADD COLUMN IF NOT EXISTS artifact_signature TEXT;

UPDATE runs SET artifact_path = NULL WHERE artifact_path IS NOT NULL;
