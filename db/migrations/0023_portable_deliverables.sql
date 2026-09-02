ALTER TABLE artifacts
    ADD COLUMN IF NOT EXISTS artifact_role TEXT NOT NULL DEFAULT 'provider_evidence',
    ADD COLUMN IF NOT EXISTS file_name TEXT NOT NULL DEFAULT 'artifact.bin',
    ADD COLUMN IF NOT EXISTS metadata JSONB NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE artifacts
    DROP CONSTRAINT IF EXISTS artifacts_run_id_sha256_key;

CREATE UNIQUE INDEX IF NOT EXISTS artifacts_run_role_digest_idx
    ON artifacts(run_id, artifact_role, sha256);

CREATE UNIQUE INDEX IF NOT EXISTS artifacts_one_source_deliverable_per_run_idx
    ON artifacts(run_id)
    WHERE artifact_role = 'source_deliverable' AND status <> 'rejected';

ALTER TABLE artifacts
    DROP CONSTRAINT IF EXISTS artifacts_role_check;

ALTER TABLE artifacts
    ADD CONSTRAINT artifacts_role_check
    CHECK (artifact_role IN ('provider_evidence', 'source_deliverable'));

ALTER TABLE runs
    ADD COLUMN IF NOT EXISTS verification_sha256 TEXT,
    ADD COLUMN IF NOT EXISTS deliverable_sha256 TEXT;

CREATE TABLE IF NOT EXISTS source_deliverables (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    run_id UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    artifact_id UUID NOT NULL UNIQUE REFERENCES artifacts(id) ON DELETE CASCADE,
    form TEXT NOT NULL CHECK (
        form IN (
            'commit_branch',
            'patch',
            'archive',
            'typed_artifact_set',
            'review_only_report'
        )
    ),
    file_name TEXT NOT NULL CHECK (length(file_name) BETWEEN 1 AND 255),
    verification_sha256 TEXT NOT NULL CHECK (verification_sha256 ~ '^[0-9a-f]{64}$'),
    base_commit TEXT NOT NULL CHECK (base_commit ~ '^[0-9a-f]{40,64}$'),
    head_commit TEXT CHECK (head_commit IS NULL OR head_commit ~ '^[0-9a-f]{40,64}$'),
    branch TEXT NOT NULL CHECK (length(branch) BETWEEN 1 AND 512),
    integration_state TEXT NOT NULL CHECK (
        integration_state IN ('not_applicable', 'ready_for_review', 'published', 'integrated')
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (run_id)
);

CREATE INDEX IF NOT EXISTS source_deliverables_corp_created_idx
    ON source_deliverables(corp_id, created_at DESC);
