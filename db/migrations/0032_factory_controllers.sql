CREATE TABLE IF NOT EXISTS factory_controllers (
    id UUID PRIMARY KEY,
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    service_actor_id UUID NOT NULL REFERENCES actors(id) ON DELETE RESTRICT,
    configured_by UUID NOT NULL REFERENCES actors(id) ON DELETE RESTRICT,
    source_project_owner TEXT NOT NULL,
    source_project_number BIGINT NOT NULL CHECK (source_project_number > 0),
    source_repository_owner TEXT NOT NULL,
    source_repository_name TEXT NOT NULL,
    desired_state TEXT NOT NULL DEFAULT 'running'
        CHECK (desired_state IN ('running', 'paused')),
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    connection_epoch UUID NOT NULL,
    lease_expires_at TIMESTAMPTZ NOT NULL,
    last_heartbeat_at TIMESTAMPTZ NOT NULL,
    reconcile_generation BIGINT NOT NULL DEFAULT 0 CHECK (reconcile_generation >= 0),
    completed_reconcile_generation BIGINT NOT NULL DEFAULT 0
        CHECK (
            completed_reconcile_generation >= 0
            AND completed_reconcile_generation <= reconcile_generation
        ),
    active_work_item_id UUID REFERENCES factory_work_items(id) ON DELETE SET NULL,
    reconcile_started_at TIMESTAMPTZ,
    last_reconciled_at TIMESTAMPTZ,
    last_reconcile_result TEXT
        CHECK (last_reconcile_result IN ('succeeded', 'failed')),
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (
        corp_id,
        source_project_owner,
        source_project_number,
        source_repository_owner,
        source_repository_name
    )
);

CREATE INDEX IF NOT EXISTS factory_controllers_corp_lease_idx
    ON factory_controllers(corp_id, lease_expires_at);

CREATE TABLE IF NOT EXISTS factory_controller_operations (
    corp_id UUID NOT NULL REFERENCES corps(id) ON DELETE CASCADE,
    controller_id UUID NOT NULL REFERENCES factory_controllers(id) ON DELETE CASCADE,
    idempotency_key TEXT NOT NULL,
    actor_id UUID NOT NULL REFERENCES actors(id) ON DELETE RESTRICT,
    operation TEXT NOT NULL CHECK (
        operation IN ('configure', 'pause', 'resume', 'reconcile')
    ),
    request JSONB NOT NULL,
    resulting_version BIGINT NOT NULL CHECK (resulting_version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (corp_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS factory_controller_operations_controller_idx
    ON factory_controller_operations(controller_id, created_at);
