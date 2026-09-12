CREATE TABLE workspace_connections (
    id uuid PRIMARY KEY,
    corp_id uuid NOT NULL REFERENCES corps(id),
    room_id uuid NOT NULL REFERENCES rooms(id),
    created_by uuid NOT NULL REFERENCES actors(id),
    runner_id text NOT NULL REFERENCES runner_nodes(id),
    label text NOT NULL CHECK (length(label) BETWEEN 1 AND 120),
    agent text NOT NULL CHECK (agent IN ('github-copilot', 'codex', 'claude-code')),
    configuration jsonb NOT NULL CHECK (jsonb_typeof(configuration) = 'object'),
    source_repository text,
    source_repository_id text,
    source_base_ref text,
    source_base_commit text,
    status text NOT NULL CHECK (
        status IN ('connecting', 'ready', 'needs_sign_in', 'not_installed',
                   'offline', 'incompatible', 'failed')
    ),
    detail text NOT NULL DEFAULT '',
    models jsonb NOT NULL DEFAULT '[]'::jsonb CHECK (jsonb_typeof(models) = 'array'),
    version bigint NOT NULL DEFAULT 1 CHECK (version > 0),
    last_checked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (corp_id, id),
    CHECK (
        (source_repository IS NULL AND source_repository_id IS NULL
         AND source_base_ref IS NULL AND source_base_commit IS NULL)
        OR
        (source_repository IS NOT NULL AND source_base_ref IS NOT NULL AND source_base_commit IS NOT NULL
         AND source_base_commit ~ '^[0-9a-f]{40}([0-9a-f]{24})?$')
    ),
    CHECK (status <> 'ready' OR source_repository IS NOT NULL)
);

CREATE INDEX workspace_connections_room
    ON workspace_connections(corp_id, room_id, created_at, id);

CREATE TABLE workspace_setup_operations (
    id uuid PRIMARY KEY,
    corp_id uuid NOT NULL REFERENCES corps(id),
    room_id uuid NOT NULL REFERENCES rooms(id),
    actor_id uuid NOT NULL REFERENCES actors(id),
    runner_id text NOT NULL REFERENCES runner_nodes(id),
    connection_id uuid,
    expected_connection_version bigint,
    idempotency_key text NOT NULL CHECK (length(idempotency_key) BETWEEN 1 AND 160),
    kind text NOT NULL CHECK (
        kind IN ('inspect_github', 'sign_in_github', 'list_github_repositories',
                 'connect', 'test', 'sign_in_agent')
    ),
    action jsonb NOT NULL CHECK (jsonb_typeof(action) = 'object'),
    request jsonb NOT NULL CHECK (jsonb_typeof(request) = 'object'),
    status text NOT NULL DEFAULT 'queued' CHECK (
        status IN ('queued', 'running', 'needs_sign_in', 'succeeded', 'failed', 'cancelled')
    ),
    report jsonb CHECK (report IS NULL OR jsonb_typeof(report) = 'object'),
    dispatch_epoch uuid,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    UNIQUE (corp_id, actor_id, idempotency_key),
    UNIQUE (corp_id, id),
    FOREIGN KEY (corp_id, connection_id) REFERENCES workspace_connections(corp_id, id),
    CHECK (expires_at > created_at),
    CHECK ((connection_id IS NULL) = (expected_connection_version IS NULL)),
    CHECK (expected_connection_version IS NULL OR expected_connection_version > 0),
    CHECK ((kind IN ('connect','test','sign_in_agent')) = (connection_id IS NOT NULL))
);

CREATE INDEX workspace_setup_pending
    ON workspace_setup_operations(runner_id, created_at, id)
    WHERE status IN ('queued', 'running', 'needs_sign_in');
CREATE UNIQUE INDEX workspace_setup_one_active_connection
    ON workspace_setup_operations(corp_id, connection_id)
    WHERE connection_id IS NOT NULL AND status IN ('queued', 'running', 'needs_sign_in');

CREATE TABLE workspace_connection_preferences (
    corp_id uuid NOT NULL REFERENCES corps(id),
    room_id uuid NOT NULL REFERENCES rooms(id),
    actor_id uuid NOT NULL REFERENCES actors(id),
    connection_id uuid NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (corp_id, room_id, actor_id),
    FOREIGN KEY (corp_id, connection_id) REFERENCES workspace_connections(corp_id, id)
);

ALTER TABLE runs ADD COLUMN workspace_connection_id uuid;
ALTER TABLE runs ADD CONSTRAINT runs_workspace_connection_scope
    FOREIGN KEY (corp_id, workspace_connection_id) REFERENCES workspace_connections(corp_id, id);
