CREATE TABLE IF NOT EXISTS factory_work_items (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('github_project_issue')),
    source_project_owner TEXT NOT NULL,
    source_project_number BIGINT NOT NULL CHECK (source_project_number > 0),
    source_project_item_id TEXT NOT NULL,
    source_repository_owner TEXT NOT NULL,
    source_repository_name TEXT NOT NULL,
    source_issue_number BIGINT NOT NULL CHECK (source_issue_number > 0),
    source_issue_node_id TEXT NOT NULL,
    source_issue_url TEXT NOT NULL,
    source_title TEXT NOT NULL,
    source_revision TEXT NOT NULL,
    state TEXT NOT NULL CHECK (
        state IN (
            'claimed',
            'mission_created',
            'running',
            'blocked',
            'awaiting_approval',
            'verification_failed',
            'verified',
            'publishing',
            'published',
            'failed',
            'cancelled'
        )
    ),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    claim_owner_id UUID NOT NULL REFERENCES actors(id) ON DELETE RESTRICT,
    claim_token UUID NOT NULL UNIQUE,
    lease_expires_at TIMESTAMPTZ NOT NULL,
    policy JSONB NOT NULL DEFAULT '{}'::jsonb,
    mission_id UUID UNIQUE REFERENCES missions(id) ON DELETE RESTRICT,
    failure_detail TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (
        corp_id,
        source_kind,
        source_project_owner,
        source_project_number,
        source_project_item_id
    )
);

CREATE INDEX IF NOT EXISTS factory_work_items_corp_state_idx
    ON factory_work_items(corp_id, state, created_at);

CREATE INDEX IF NOT EXISTS factory_work_items_lease_idx
    ON factory_work_items(corp_id, lease_expires_at)
    WHERE state = 'claimed';

CREATE TABLE IF NOT EXISTS factory_operations (
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    idempotency_key TEXT NOT NULL,
    work_item_id UUID NOT NULL REFERENCES factory_work_items(id) ON DELETE CASCADE,
    actor_id UUID NOT NULL REFERENCES actors(id) ON DELETE RESTRICT,
    operation TEXT NOT NULL CHECK (operation IN ('claim', 'renew', 'materialize')),
    resulting_version BIGINT NOT NULL CHECK (resulting_version > 0),
    claim_token UUID,
    request JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (corp_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS factory_operations_work_item_idx
    ON factory_operations(work_item_id, created_at);
