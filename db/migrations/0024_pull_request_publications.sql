CREATE TABLE IF NOT EXISTS pull_request_publications (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    factory_work_item_id UUID NOT NULL UNIQUE REFERENCES factory_work_items(id) ON DELETE RESTRICT,
    mission_id UUID NOT NULL REFERENCES missions(id) ON DELETE RESTRICT,
    source_deliverable_id UUID NOT NULL UNIQUE REFERENCES source_deliverables(id) ON DELETE RESTRICT,
    artifact_id UUID NOT NULL REFERENCES artifacts(id) ON DELETE RESTRICT,
    task_id UUID NOT NULL REFERENCES tasks(id) ON DELETE RESTRICT,
    run_id UUID NOT NULL REFERENCES runs(id) ON DELETE RESTRICT,
    source_issue_number BIGINT NOT NULL CHECK (source_issue_number > 0),
    source_issue_url TEXT NOT NULL,
    target_repository TEXT NOT NULL CHECK (length(target_repository) BETWEEN 3 AND 255),
    base_ref TEXT NOT NULL CHECK (length(base_ref) BETWEEN 1 AND 512),
    branch TEXT NOT NULL CHECK (length(branch) BETWEEN 1 AND 512),
    commit_sha TEXT NOT NULL CHECK (commit_sha ~ '^[0-9a-f]{40,64}$'),
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 256),
    body TEXT NOT NULL CHECK (length(body) BETWEEN 1 AND 65536),
    actor_id UUID NOT NULL REFERENCES actors(id) ON DELETE RESTRICT,
    authorization_id UUID NOT NULL,
    authorization JSONB NOT NULL,
    effect_key TEXT NOT NULL CHECK (length(effect_key) BETWEEN 1 AND 500),
    idempotency_key TEXT NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 500),
    state TEXT NOT NULL CHECK (
        state IN ('requested', 'publishing', 'branch_pushed', 'pull_request_created', 'published')
    ),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    publisher_id TEXT,
    publisher_token UUID UNIQUE,
    publisher_lease_expires_at TIMESTAMPTZ,
    failure_detail TEXT,
    branch_pushed_at TIMESTAMPTZ,
    pull_request_number BIGINT CHECK (
        pull_request_number IS NULL OR pull_request_number > 0
    ),
    pull_request_node_id TEXT,
    pull_request_url TEXT,
    pull_request_state TEXT,
    pull_request_draft BOOLEAN,
    project_owner TEXT NOT NULL,
    project_number BIGINT NOT NULL CHECK (project_number > 0),
    project_item_id TEXT NOT NULL,
    project_status_before TEXT NOT NULL,
    project_status_after TEXT,
    project_status_updated_at TIMESTAMPTZ,
    auto_merge_enabled BOOLEAN NOT NULL DEFAULT FALSE CHECK (NOT auto_merge_enabled),
    merge_authorized BOOLEAN NOT NULL DEFAULT FALSE CHECK (NOT merge_authorized),
    deployment_authorized BOOLEAN NOT NULL DEFAULT FALSE CHECK (NOT deployment_authorized),
    provenance JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (corp_id, effect_key),
    UNIQUE (corp_id, idempotency_key),
    UNIQUE (corp_id, target_repository, branch),
    CHECK (
        (publisher_id IS NULL AND publisher_token IS NULL AND publisher_lease_expires_at IS NULL)
        OR
        (publisher_id IS NOT NULL AND publisher_token IS NOT NULL AND publisher_lease_expires_at IS NOT NULL)
    ),
    CHECK (
        pull_request_number IS NULL
        OR (
            pull_request_node_id IS NOT NULL
            AND pull_request_url IS NOT NULL
            AND pull_request_state IS NOT NULL
            AND pull_request_draft IS NOT NULL
        )
    )
);

CREATE INDEX IF NOT EXISTS pull_request_publications_corp_state_idx
    ON pull_request_publications(corp_id, state, created_at DESC);

CREATE TABLE IF NOT EXISTS pull_request_publication_attempts (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    publication_id UUID NOT NULL REFERENCES pull_request_publications(id) ON DELETE CASCADE,
    attempt INTEGER NOT NULL CHECK (attempt > 0),
    actor_id UUID NOT NULL REFERENCES actors(id) ON DELETE RESTRICT,
    publisher_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('running', 'failed', 'abandoned', 'published')),
    failure_detail TEXT,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ,
    UNIQUE (publication_id, attempt)
);

CREATE INDEX IF NOT EXISTS pull_request_publication_attempts_corp_idx
    ON pull_request_publication_attempts(corp_id, publication_id, attempt);

CREATE TABLE IF NOT EXISTS pull_request_publication_operations (
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    idempotency_key TEXT NOT NULL,
    publication_id UUID NOT NULL REFERENCES pull_request_publications(id) ON DELETE CASCADE,
    actor_id UUID NOT NULL REFERENCES actors(id) ON DELETE RESTRICT,
    operation TEXT NOT NULL CHECK (
        operation IN ('start', 'renew', 'branch_pushed', 'pull_request_created', 'published', 'failed')
    ),
    resulting_version BIGINT NOT NULL CHECK (resulting_version > 0),
    publisher_token UUID,
    request JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (corp_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS pull_request_publication_operations_publication_idx
    ON pull_request_publication_operations(publication_id, created_at);
